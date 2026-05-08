use v8::{cppgc::GarbageCollected};

use crate::{extension::{NativeObject, NativeObjectBuilder}};


/// Basic console API for testing
pub struct Console {}

impl Console {
    /// Creates a new Stream object
    pub fn new() -> Self {
        Self {}
    }
}

unsafe impl GarbageCollected for Console {
    fn get_name(&self) -> &'static std::ffi::CStr {
        c"Console"
    }

    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {
        // No fields to trace
    }
}

impl NativeObject for Console {
    fn class_name() -> &'static str {
        "Console"
    }

    fn constructor<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, _retval: v8::ReturnValue) {
        let s = Self {};
        s.finalize_constructor(scope, args);
    }

    fn bind_methods<'a, 's, 'i>(builder: NativeObjectBuilder) {
        builder
        .method("log", console_log);
    }
}

fn console_log<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    _retval: v8::ReturnValue,
) {
    let arg_len = args.length();
    let mut v = Vec::new();
    for arg in 0..arg_len {
        let arg = args.get(arg);
        v.push(pretty_print_arg(scope, arg));
    }

    println!("{}", v.join(", "))
}

fn pretty_print_arg<'s>(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'_, v8::Value>) -> String {
    if let Some(v) = value.to_detail_string(scope) {
        let s = v.to_rust_string_lossy(scope);
        return s;
    } else {
        return "undefined".to_string()
    }
}