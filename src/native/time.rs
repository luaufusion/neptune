use v8::MapFnTo;

use crate::{extension::Globals, state::IsolateState, timer::ItemHandler};

fn set_timeout<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) {
    // Extract out callbacj first
    let callback = match v8::Local::<v8::Function>::try_from(args.get(0)) {
        Ok(cb) => cb,
        Err(_) => {
            let msg = v8::String::new(scope, "First argument to setTimeout must be a function").unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return;
        }
    };

    let delay_ms = args.get(1).to_uint32(scope).map(|v| v.value()).unwrap_or(4).max(4);

    let mut extra_args = Vec::with_capacity((args.length() - 2) as usize);
    for i in 2..args.length() {
        extra_args.push(v8::Global::new(scope, args.get(i)));
    }

    let handler = ItemHandler::Call {
        cb: v8::Global::new(scope, callback),
        args: extra_args, 
    };

    let id = IsolateState::with_mut(scope, |state| {
        state.queue_stream_mut().add(handler, std::time::Duration::from_millis(delay_ms as u64))
    });

    retval.set(v8::Integer::new(scope, id as i32).into());
}

fn set_interval<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) {
    // Extract out callbacj first
    let callback = match v8::Local::<v8::Function>::try_from(args.get(0)) {
        Ok(cb) => cb,
        Err(_) => {
            let msg = v8::String::new(scope, "First argument to setInterval must be a function").unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return;
        }
    };

    let delay_ms = args.get(1).to_uint32(scope).map(|v| v.value()).unwrap_or(4).max(4);

    let mut extra_args = Vec::with_capacity((args.length() - 2) as usize);
    for i in 2..args.length() {
        extra_args.push(v8::Global::new(scope, args.get(i)));
    }

    let handler = ItemHandler::RepeatCall {
        cb: v8::Global::new(scope, callback),
        args: extra_args, 
    };

    let id = IsolateState::with_mut(scope, |state| {
        state.queue_stream_mut().add(handler, std::time::Duration::from_millis(delay_ms as u64))
    });

    retval.set(v8::Integer::new(scope, id as i32).into());
}

// The logic for clearInterval and clearTimeout are the same
fn clear_timer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue,
) {
    let val = args.get(0);
    if !val.is_number() {
        return;
    }

    let id = val.to_uint32(scope).unwrap().value() as u64;

    // 2. Access IsolateState and cancel the timer
    IsolateState::with_mut(scope, |state| {
        // We don't need the returned Item, so we just let it drop
        state.queue_stream_mut().cancel(id);
    });
}

pub struct TimerGlobals {}
impl Globals for TimerGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        Self::add(scope, global, "setTimeout", set_timeout);
        Self::add(scope, global, "setInterval", set_interval);
        Self::add(scope, global, "clearInterval", clear_timer);
        Self::add(scope, global, "clearTimeout", clear_timer);
    }

    fn get_external_references() -> Vec<v8::FunctionCallback> {
        vec![set_timeout.map_fn_to(), set_interval.map_fn_to(), clear_timer.map_fn_to(), clear_timer.map_fn_to()]
    }
}

fn performance_now<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut retval: v8::ReturnValue,
) {
    let elapsed = IsolateState::with(scope, |state| state.elapsed_ms());
    retval.set(v8::Number::new(scope, elapsed).into());
}

pub struct PerformanceGlobals;

impl Globals for PerformanceGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        let performance_key = v8::String::new(scope, "performance").unwrap();
        let performance_obj = v8::Object::new(scope);
        
        let now_key = v8::String::new(scope, "now").unwrap();
        let now_tmpl = v8::FunctionTemplate::new(scope, performance_now);

        performance_obj.set(scope, now_key.into(), now_tmpl.get_function(scope).unwrap().into());
        global.set(scope, performance_key.into(), performance_obj.into());
    }

    fn get_external_references() -> Vec<v8::FunctionCallback> {
        vec![performance_now.map_fn_to()]
    }
}
