use std::{borrow::Cow, collections::{HashMap, VecDeque}};

const PRIMORDIALS: &str = include_str!("node/primordials.js");
const SCRIPT_EXEC_WRAPPER: &str = include_str!("neptune/script_exec_wrapper.js"); // script wrapper to execute all of neptunes js code correctly

/// Load allows loading in scripts into the created neptune runtime
/// 
/// It supports both snapshotted and non-snapshotted runtimes
pub fn load_js<'s>(scope: &mut v8::PinScope<'s, '_>, files: Vec<(Cow<'static, str>, Cow<'static, str>)>) -> Result<(), crate::Error> {
    let mut files = VecDeque::from(files);
    // insert primordials in the first spot of our vec
    files.push_front((Cow::Borrowed("node/primordials.js"), Cow::Borrowed(PRIMORDIALS)));

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

    Ok(())
}
