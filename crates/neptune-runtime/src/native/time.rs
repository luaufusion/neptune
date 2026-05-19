use neptune_macros::op;

use crate::{extension::{Globals, NeptuneError}, state_mut, state_ref, timer::ItemHandler};

#[op(raw)]
fn set_timeout<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) -> Result<(), NeptuneError> {
    // Extract out callbacj first
    let callback = match v8::Local::<v8::Function>::try_from(args.get(0)) {
        Ok(cb) => cb,
        Err(_) => {
            let msg = v8::String::new(scope, "First argument to setTimeout must be a function").unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return Ok(());
        }
    };

    let delay_ms = args.get(1).to_uint32(scope).map(|v| v.value()).unwrap_or(4).max(4);

    let length = args.length() as i32;
    let extra_count = (length - 2).max(0) as usize;
    let mut extra_args = Vec::with_capacity(extra_count);
    for i in 2..length {
        extra_args.push(v8::Global::new(scope, args.get(i)));
    }

    let handler = ItemHandler::Call {
        cb: v8::Global::new(scope, callback),
        args: extra_args, 
    };

    state_mut!(let state, scope);
    let id = state.queue_stream_mut().add(handler, std::time::Duration::from_millis(delay_ms as u64), false);

    retval.set(v8::Integer::new(scope, id as i32).into());
    Ok(())
}

#[op(raw)]
fn set_interval<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) -> Result<(), NeptuneError> {
    // Extract out callbacj first
    let callback = match v8::Local::<v8::Function>::try_from(args.get(0)) {
        Ok(cb) => cb,
        Err(_) => {
            let msg = v8::String::new(scope, "First argument to setInterval must be a function").unwrap();
            let exception = v8::Exception::type_error(scope, msg);
            scope.throw_exception(exception);
            return Ok(());
        }
    };

    let delay_ms = args.get(1).to_uint32(scope).map(|v| v.value()).unwrap_or(4).max(4);

    let length = args.length() as i32;
    let extra_count = (length - 2).max(0) as usize;
    let mut extra_args = Vec::with_capacity(extra_count);
    for i in 2..length {
        extra_args.push(v8::Global::new(scope, args.get(i)));
    }

    let handler = ItemHandler::Call {
        cb: v8::Global::new(scope, callback),
        args: extra_args, 
    };

    state_mut!(let state, scope);
    let id = state.queue_stream_mut().add(handler, std::time::Duration::from_millis(delay_ms as u64), true);

    retval.set(v8::Integer::new(scope, id as i32).into());
    Ok(())
}

// The logic for clearInterval and clearTimeout are the same
#[op]
fn clear_timer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: u32, // TODO: fix this
) -> Result<(), NeptuneError> {
    state_mut!(let state, scope);
    state.queue_stream_mut().cancel(id as u64);
    
    Ok(())
}

neptune_macros::define_globals! {
    pub struct TimerGlobals {
        setTimeout: set_timeout,
        setInterval: set_interval,
        clearTimeout: clear_timer,
        clearInterval: clear_timer,
    }
}

#[op]
fn performance_now<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<f64, NeptuneError> {
    state_ref!(let state, scope);
    Ok(state.elapsed_ms())
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

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        use v8::MapFnTo;
        vec![("performance.now", performance_now.map_fn_to())]
    }
}
