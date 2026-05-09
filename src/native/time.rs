use std::time::Duration;

use tokio::{sync::Mutex, time::Instant};
use v8::{cppgc::GarbageCollected};

use crate::{extension::{MethodBuilder, NativeObject, NativeObjectBuilder}, runtime::Value};


pub struct Ticker {
    timer: Mutex<tokio::time::Interval>,
    start: Instant
}

impl Ticker {
    /// Creates a new Ticker object
    pub fn new(s: u64) -> Self {
        let dur = Duration::from_millis(s);
        let start = Instant::now();
        let timer = tokio::time::interval_at(start + dur, dur).into();
        Self { timer, start }
    }
}

unsafe impl GarbageCollected for Ticker {
    fn get_name(&self) -> &'static std::ffi::CStr {
        c"Ticker"
    }

    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {
        // No fields to trace
    }
}

impl NativeObject for Ticker {
    fn class_name() -> &'static str {
        "Ticker"
    }

    fn constructor<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, _retval: v8::ReturnValue) {
        let arg0 = args.get(0);
        let Some(n) = arg0.to_number(scope) else {
            let msg = v8::String::new(scope, "first argument must be number of milliseconds to tick at").unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);   
            return;
        };
        let n = n.uint32_value(scope).unwrap_or(1) as u64;

        let s = Self::new(n.max(1));
        s.finalize_constructor(scope, args);
    }

    fn bind_methods<'a, 's, 'i>(builder: NativeObjectBuilder) {
        builder
        .method("tick", ticker_tick);
    }
}

fn ticker_tick<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    MethodBuilder::bind_async_method::<Ticker, _, _>(
        scope,
        args,
        retval,
        |bridge, _scope, _args| {
            Ok(async move {
                let mut ticker = bridge.timer.lock().await;
                let t = ticker.tick().await;
                let elapsed = t - bridge.start;
                Ok(Value::F64(elapsed.as_secs_f64()*1000.0))
            })
        },
    );
}