use std::future::Future;
use crate::cppgc::{try_unwrap_cppgc_persistent_object, Ref};
use crate::runtime::{AsyncResult, IsolateState, Value as OpValue};

type Cb = for<'s> fn(&mut v8::PinScope<'s, '_>, v8::FunctionCallbackArguments<'s>, v8::ReturnValue);

pub struct NativeObjectCallbacks {
    constructor: Cb,
    funcs: Vec<(&'static str, Cb)>,
} 

impl NativeObjectCallbacks {
    pub fn no_op<'s>(scope: &mut v8::PinScope<'s, '_>, _args: v8::FunctionCallbackArguments, _retval: v8::ReturnValue) {
        let msg = v8::String::new(scope, "Illegal constructor").unwrap();
        let exception = v8::Exception::type_error(scope, msg);
        scope.throw_exception(exception);
    }

    pub fn new(constructor: Cb) -> Self {
        Self {
            constructor,
            funcs: Vec::new(),
        }
    }

    pub fn method(mut self, name: &'static str, cb: Cb) -> Self {
        self.funcs.push((name, cb));
        self
    }

    pub fn new_noconstructor() -> Self {
        Self::new(Self::no_op)
    }
}

/// A native cppgc'able v8 object
pub trait NativeObject: v8::cppgc::GarbageCollected + Sized + 'static {
    fn class_name() -> &'static str;
    fn define_callbacks() -> NativeObjectCallbacks;

    /// Set up the base cppgc template
    fn setup<'s>(scope: &mut v8::PinScope<'s, '_, ()>) -> v8::Local<'s, v8::FunctionTemplate> {
        let cbs_obj = Self::define_callbacks();
        let constructor_tpl = v8::FunctionTemplate::new(scope, cbs_obj.constructor);
        
        // Set the class name (This makes `console.log(obj)` print `ClassName { ... }`
        let class_name = v8::String::new(scope, Self::class_name()).unwrap();
        constructor_tpl.set_class_name(class_name);

        // Configure the Instance Template (The actual objects created)
        //
        // Note that we need 2 internal fields for cppgc!
        let instance_tpl = constructor_tpl.instance_template(scope);
        instance_tpl.set_internal_field_count(2);

        // Configure the Prototype Template (The methods)
        let prototype_tpl = constructor_tpl.prototype_template(scope);
        
        for (name, cb) in cbs_obj.funcs {
            let method_name = v8::String::new(scope, name).unwrap();
            let method_callback = v8::FunctionTemplate::new(scope, cb);
        
            // Attach the method to the prototype!
            prototype_tpl.set(method_name.into(), method_callback.into());
        }

        constructor_tpl
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

                let (tx, promise_id) = {
                    let state = scope.get_slot_mut::<IsolateState>().expect("IsolateState missing");
                    // Get next promise id to use
                    let id = state.next_promise_id;
                    state.next_promise_id += 1;
                    
                    // Increment pending work so event loop knows not to exit
                    state.pending_promises += 1;
                    
                    // Store the V8 Promise in the registry so the Tokio task can resolve it later
                    state.promise_registry.insert(id, global_resolver);
                    
                    (state.tx.clone(), id)
                };
                
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