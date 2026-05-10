use std::future::Future;
use crate::cppgc::{try_unwrap_cppgc_persistent_object, Ref};
use crate::state::{AsyncResult, IsolateState};
use crate::runtime::{Value as OpValue};

/// An internal builder struct used to define methods on a NativeObject
pub struct NativeObjectBuilder<'a, 's, 'i> {
    scope: &'a mut v8::PinScope<'s, 'i, ()>, 
    prototype: v8::Local<'s, v8::ObjectTemplate>, 
}

impl<'a, 's, 'i> NativeObjectBuilder<'a, 's, 'i> {
    pub fn method<F>(
        self,
        name: &str, 
        callback: F 
    ) -> Self 
    where
        F: v8::MapFnTo<v8::FunctionCallback> 
    {
        let method_name = v8::String::new(self.scope, name).unwrap();
        let method_callback = v8::FunctionTemplate::new(self.scope, callback);
        self.prototype.set(method_name.into(), method_callback.into());
        self
    }
}

/// A native cppgc'able v8 object
pub trait NativeObject: v8::cppgc::GarbageCollected + Sized + 'static {
    fn class_name() -> &'static str;
    fn constructor<'s>(scope: &mut v8::PinScope<'s, '_>, _args: v8::FunctionCallbackArguments<'s>, _retval: v8::ReturnValue) {
        let msg = v8::String::new(scope, "Illegal constructor").unwrap();
        let exception = v8::Exception::type_error(scope, msg);
        scope.throw_exception(exception);
    }

    /// Binds methods to the native object
    fn bind_methods<'a, 's, 'i>(_builder: NativeObjectBuilder) {}

    /// Set up the base cppgc template
    fn setup<'s>(scope: &mut v8::PinScope<'s, '_, ()>) -> v8::Local<'s, v8::FunctionTemplate> {
        let constructor_tpl = v8::FunctionTemplate::new(scope, Self::constructor);

        // Set the class name (This makes `console.log(obj)` print `ClassName { ... }`
        let class_name = v8::String::new(scope, Self::class_name()).unwrap();
        constructor_tpl.set_class_name(class_name);

        // Configure the instance template (The actual objects created)
        //
        // Note that we need 2 internal fields for cppgc!
        let instance_tpl = constructor_tpl.instance_template(scope);
        instance_tpl.set_internal_field_count(2);

        // Configure the Prototype Template (The methods)
        let prototype_tpl = constructor_tpl.prototype_template(scope);

        let builder = NativeObjectBuilder { scope, prototype: prototype_tpl };
        Self::bind_methods(builder);

        constructor_tpl
    }

    /// Helper to finalize the binding of a Rust struct to a JS object in a constructor.
    fn finalize_constructor(
        self,
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
    ) {
        // Ensure this is actually in a constructor
        if !args.is_construct_call() {
            let msg = v8::String::new(scope, &format!("Class constructor {} cannot be invoked without 'new'", Self::class_name())).unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return;
        }

        // Bind to 'this'
        let js_this = args.this();

        // Safety check before binding
        if js_this.internal_field_count() != 2 {
            let msg = v8::String::new(scope, &format!("Illegal internal field count in constructor for {}", Self::class_name())).unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return;
        }
        crate::cppgc::wrap_object(scope, js_this, self);
    }
}

pub struct MethodBuilder;

impl MethodBuilder {
    /// A generic builder for sync prototype methods.
    pub fn bind_sync_method<'s, T, Func>(
        scope: &mut v8::PinScope<'s, '_>,
        args: v8::FunctionCallbackArguments<'s>,
        mut retval: v8::ReturnValue,
        func: Func, 
    ) 
    where
        T: v8::cppgc::GarbageCollected + 'static,
        Func: FnOnce(Ref<T>, &mut v8::PinScope<'s, '_>, v8::FunctionCallbackArguments<'s>) -> Result<OpValue, crate::Error>,
    {
        let js_this = args.this();

        let pref = match try_unwrap_cppgc_persistent_object::<T>(scope.as_mut(), js_this.into()) {
            Some(ptr) => ptr,
            None => {
                let err = v8::String::new(scope, "TypeError: Illegal invocation").unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, err));
                return;
            }
        };

        let resp = func(pref, scope, args);

        match resp {
            Ok(val) => {
                let v8_result = val.to_v8(scope);
                retval.set(v8_result);

            }
            Err(sync_err) => {
                // Function failed (e.g., bad arguments passed from JS)
                let err = v8::String::new(scope, &sync_err.to_string()).unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, err));
            }
        }
    }

    /// A generic builder for async prototype methods.
    pub fn bind_async_method<'s, T, ExtractFn, Fut>(
        scope: &mut v8::PinScope<'s, '_>,
        args: v8::FunctionCallbackArguments<'s>,
        mut retval: v8::ReturnValue,
        // The closure that runs synchronously to extract data and return a Future
        extractor: ExtractFn, 
    ) 
    where
        T: v8::cppgc::GarbageCollected + 'static,
        ExtractFn: FnOnce(Ref<T>, &mut v8::PinScope<'s, '_>, v8::FunctionCallbackArguments<'s>) -> Result<Fut, crate::Error>,
        Fut: Future<Output = Result<OpValue, crate::Error>> + 'static,
    {
        let js_this = args.this();

        let pref = match try_unwrap_cppgc_persistent_object::<T>(scope.as_mut(), js_this.into()) {
            Some(ptr) => ptr,
            None => {
                let err = v8::String::new(scope, "TypeError: Illegal invocation").unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, err));
                return;
            }
        };

        let future_result = extractor(pref, scope, args);

        match future_result {
            Ok(future) => {
                let resolver = v8::PromiseResolver::new(scope).unwrap();
                let promise = resolver.get_promise(scope);
                let global_resolver = v8::Global::new(scope, resolver);

                let (tx, promise_id) = IsolateState::with_mut(scope, |state| {
                    state.attach_to_scheduler(global_resolver)           
                });
                
                tokio::task::spawn_local(async move {
                    let result = future.await;
                    tx.send(AsyncResult { promise_id, result }).unwrap();
                });

                retval.set(promise.into());
            }
            Err(sync_err) => {
                // Extraction failed (e.g., bad arguments passed from JS)
                let err = v8::String::new(scope, &sync_err.to_string()).unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, err));
            }
        }
    }
}

/// Defines a set of globals
pub trait Globals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>);

    /// Adds a global to global scope
    fn add<'s, F>(
        scope: &mut v8::PinScope<'s, '_>, 
        global: v8::Local<v8::Object>,
        name: &str, 
        callback: F 
    ) 
    where
        F: v8::MapFnTo<v8::FunctionCallback> 
    {
        let name = v8::String::new(scope, name).unwrap();
        let tmpl = v8::FunctionTemplate::new(scope, callback);
        let val = tmpl.get_function(scope).unwrap();
        global.set(scope, name.into(), val.into());
    }
}

pub trait SnapshottableGlobals: Globals {
    fn get_external_references() -> Vec<v8::FunctionCallback>;
}