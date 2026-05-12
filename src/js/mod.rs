use std::{borrow::Cow, collections::{HashMap, VecDeque}};

use v8::MapFnTo;

use crate::{extension::Globals, runtime::{ConsoleLogMode, LogMessage}, state::IsolateState};

// node
const PRIMORDIALS: &str = include_str!("node/primordials.js");

// neptune
const SCRIPT_EXEC_WRAPPER: &str = include_str!("neptune/script_exec_wrapper.js"); // script wrapper to execute all of neptunes js code correctly
const CONSOLE_JS: &str = include_str!("neptune/console.js");

// Internal bootstrap function to probe a promise 
//
// On success, returns an array: [state, value/reason]
fn bootstrap_get_promise_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let promise = match v8::Local::<v8::Promise>::try_from(args.get(0)) {
        Ok(p) => p,
        Err(_) => {
            let Some(msg) = v8::String::new(scope, "Promise expected as argument to getPromiseDetails") else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let state = promise.state();
    let state_int = match state {
        v8::PromiseState::Pending => 0,
        v8::PromiseState::Fulfilled => 1,
        v8::PromiseState::Rejected => 2,
    };

    let result_arr = v8::Array::new(scope, 2);
    let state_val = v8::Integer::new(scope, state_int);
    result_arr.set_index(scope, 0, state_val.into());

    if state != v8::PromiseState::Pending {
        let value = promise.result(scope);
        result_arr.set_index(scope, 1, value);
    }

    retval.set(result_arr.into());
}

fn bootstrap_console_log<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    _retval: v8::ReturnValue,
) {
    let mode = match v8::Local::<v8::Uint32>::try_from(args.get(0)) {
        Ok(p) => p,
        Err(_) => {
            let Some(msg) = v8::String::new(scope, "Mode expected as argument to consoleLog") else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let s = match v8::Local::<v8::String>::try_from(args.get(1)) {
        Ok(p) => p,
        Err(_) => {
            let Some(msg) = v8::String::new(scope, "String expected as argument to consoleLog") else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let clm = match mode.value() {
        0 => ConsoleLogMode::Log,
        1 => ConsoleLogMode::Error,
        _ => {
            let Some(msg) = v8::String::new(scope, "Invalid mode provided") else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let s = s.to_rust_string_lossy(scope);

    IsolateState::with(scope, |state| {
        if let Some(ref cb) = state.embedder_log_cb {
            (cb)(LogMessage::ConsoleLog { msg: s, mode: clm});
        }
    });
}

pub struct BootstrapGlobals {}
impl Globals for BootstrapGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        let bobj = v8::Object::new(scope);

        Self::add(scope, bobj, "getPromiseDetails", bootstrap_get_promise_details);
        Self::add(scope, bobj, "consoleLog", bootstrap_console_log);

        let bkey = v8::String::new(scope, "bootstrap").unwrap();
        global.set(scope, bkey.into(), bobj.into());
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![("getPromiseDetails", bootstrap_get_promise_details.map_fn_to()), ("consoleLog", bootstrap_console_log.map_fn_to())]
    }

    fn js_files() -> Vec<(Cow<'static, str>, Cow<'static, str>)> {
        // we need primordials first
        vec![
            (Cow::Borrowed("node/primordials.js"), Cow::Borrowed(PRIMORDIALS)),
            (Cow::Borrowed("node/console.js"), Cow::Borrowed(CONSOLE_JS))
        ]
    }
}

/// Load allows loading in scripts into the created neptune runtime
/// 
/// It supports both snapshotted and non-snapshotted runtimes
/// 
/// Assumes that `BootstrapGlobals` has been loaded as the first global into the RuntimeSnapshotter before execution so primordials etc get loaded first
pub fn load_js<'s>(scope: &mut v8::PinScope<'s, '_>, files: Vec<(Cow<'static, str>, Cow<'static, str>)>) -> Result<(), crate::Error> {
    let files = VecDeque::from(files);

    // script exec wrapper needs files as a json and the fileorder
    let file_order = files.iter().map(|(k, _)| k.to_owned()).collect::<Vec<_>>();
    let files: HashMap<Cow<'static, str>, Cow<'static, str>> = HashMap::from_iter(files);

    let file_order = serde_json::to_string(&file_order)?;
    let files = serde_json::to_string(&files)?;

    // call script exec wrapper with the file order and files
    v8::tc_scope!(let scope, scope);

    let code = v8::String::new(scope, SCRIPT_EXEC_WRAPPER).unwrap();

    let script = match v8::Script::compile(scope, code, None) {
        Some(s) => s,
        None => {
            if let Some(exception) = scope.exception() {
                let msg = exception.to_rust_string_lossy(scope);
                return Err(format!("Failed to compile: {msg}").into())
            } else {
                return Err("Unknown error has occurred".into())
            } 
        }
    };
    
    if let Some(value) = script.run(scope) {
        let func = v8::Local::<v8::Function>::try_from(value)?;

        let file_order = v8::String::new(scope, &file_order).unwrap();
        let files = v8::String::new(scope, &files).unwrap();

        if func.call(scope, v8::undefined(scope).into(), &[files.into(), file_order.into()]).is_none() {
            if let Some(exception) = scope.exception() {
                let msg = exception.to_rust_string_lossy(scope);
                return Err(format!("Failed to execute funcs: {msg}").into())
            } else {
                return Err("Unknown error has occurred".into())
            } 
        }
    } else {
        if let Some(exception) = scope.exception() {
            let msg = exception.to_rust_string_lossy(scope);
            return Err(format!("Failed to execute: {msg}").into())
        } else {
            return Err("Unknown error has occurred".into())
        } 
    }

    // Drop bootstrap obj
    let bkey = v8::String::new(scope, "bootstrap").unwrap();
    scope.get_current_context().global(scope).set(scope, bkey.into(), v8::undefined(scope).into());

    Ok(())
}
