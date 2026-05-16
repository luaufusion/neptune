use std::time::Duration;

use v8::MapFnTo;

use crate::extension::{Globals, wrap_async};

fn sleep_async<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap_async(scope, args, retval, |_scope, (time,): (i32,)| {
        Ok(async move {
            tokio::time::sleep(Duration::from_secs(time as u64)).await;
            Ok(12)
        })
    });
}

pub struct TestGlobals {}
impl Globals for TestGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        Self::add(scope, global, "testSleepAsync", sleep_async);
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![
            ("testSleepAsync", sleep_async.map_fn_to()), 
        ]
    }
}
