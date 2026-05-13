use std::borrow::Cow;
use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Helper method to wrap a function
pub fn wrap_raw<'s, Func, E>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    retval: v8::ReturnValue,
    func: Func, 
) 
    where Func: FnOnce(&mut v8::PinScope<'s, '_>, v8::FunctionCallbackArguments<'s>, v8::ReturnValue) -> Result<(), E>,
    E: Display
{
    let res = catch_unwind(AssertUnwindSafe(|| func(scope, args, retval)));
    match res {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            let Some(msg) = v8::String::new(scope, &e.to_string()) else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
        Err(p) => {
            let err_msg = {
                // If downcastable to String, use it
                if let Some(s) = p.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = p.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    // Otherwise, use the debug representation
                    format!("Panic occurred in callback: {:?}", p)
                }
            };

            std::mem::forget(catch_unwind(AssertUnwindSafe(move || drop(p))));

            let Some(msg) = v8::String::new(scope, &err_msg) else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    }
}

/// Defines a set of globals
pub trait Globals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>);

    /// Any external references needed for snapshotting
    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)>;

    /// Adds a global to global scope
    /// 
    /// Should not be overriden
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
        val.set_name(name.into());
        global.set(scope, name.into(), val.into());
    }

    /// Gets the bootstrap object
    fn get_bootstrap<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) -> v8::Local<'s, v8::Object> {
        let name = v8::String::new(scope, "bootstrap").unwrap();
        let bobj = global.get(scope, name.into()).unwrap();
        v8::Local::<v8::Object>::try_from(bobj).unwrap()

    }   

    /// What js files to load (if any)
    fn js_files() -> Vec<(Cow<'static, str>, Cow<'static, str>)> {
        vec![]
    }
}