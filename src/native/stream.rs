use v8::MapFnTo;

use crate::{buffer::v8_backing_store_to_vec, extension::Globals, runtime::PipedMessage, state::IsolateState};

fn post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    _retval: v8::ReturnValue,
) {
    let arg0 = args.get(0);
    
    if arg0.is_array_buffer_view() {
        let view = v8::Local::<v8::ArrayBufferView>::try_from(arg0).unwrap();
        let ab = view.buffer(scope).expect("View has no buffer");
        
        let offset = view.byte_offset();
        let len = view.byte_length();
        
        // Slice the backing store using the view's offset and length
        let dest = v8_backing_store_to_vec(ab.get_backing_store(), offset, len);

        IsolateState::with_mut(scope, |state| {
            if let Some(cb) = &mut state.worker_to_embedder_cb {
                (cb)(PipedMessage::PostedBytes(dest))
            }
        });
    } else if arg0.is_array_buffer() {
        let ab = v8::Local::<v8::ArrayBuffer>::try_from(arg0).unwrap();
        let len = ab.byte_length();
        let dest = v8_backing_store_to_vec(ab.get_backing_store(), 0, len);

        IsolateState::with_mut(scope, |state| {
            if let Some(cb) = &mut state.worker_to_embedder_cb {
                (cb)(PipedMessage::PostedBytes(dest))
            }
        });
    } else if arg0.is_string() {
        let s= v8::Local::<v8::String>::try_from(arg0).unwrap();
        let dest = s.to_rust_string_lossy(scope);

        IsolateState::with_mut(scope, |state| {
            if let Some(cb) = &mut state.worker_to_embedder_cb {
                (cb)(PipedMessage::PostedString(dest))
            }
        });
    } else {
        let Some(msg) = v8::String::new(scope, "Either ArrayBufferView/ArrayBuffer/String expected") else {
            return;
        };
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    };
}

fn set_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    _retval: v8::ReturnValue,
) {
    let arg0 = args.get(0);
    
    if arg0.is_function() {
        let cb_func = v8::Local::<v8::Function>::try_from(arg0).unwrap();
        if cb_func.is_api_wrapper() || cb_func.internal_field_count() != 0 {
            let Some(msg) = v8::String::new(scope, "Function must not be a internal API object") else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
        
        let cb_func = v8::Global::new(scope, cb_func);

        IsolateState::with_mut(scope, |state| {
            state.embedder_to_worker_cb = Some(cb_func);
        });
    } else if arg0.is_null_or_undefined() {
        IsolateState::with_mut(scope, |state| {
            state.embedder_to_worker_cb = None;
        });
    } else {
        let Some(msg) = v8::String::new(scope, "Function (or null/undefined) expected") else {
            return;
        };
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    };
}

pub struct EmbedderPipeGlobals {}
impl Globals for EmbedderPipeGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        Self::add(scope, global, "postMessage", post_message);
        Self::add(scope, global, "setMessageCallback", set_message_callback);
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![("postMessage", post_message.map_fn_to()), ("setMessageCallback", set_message_callback.map_fn_to())]
    }
}
