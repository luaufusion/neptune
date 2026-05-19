use std::time::Duration;

use crate::extension::wrap_async;

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

neptune_macros::define_globals! {
    pub struct TestGlobals {
        testSleepAsync: sleep_async,
    }
}
