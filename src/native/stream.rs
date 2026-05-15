use v8::MapFnTo;

use crate::{extension::{Globals, NeptuneError, Skip, StringOrBuffer, wrap}, runtime::PipedMessage, state::IsolateState};

fn post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap(scope, args, retval, |scope, (msg,): (StringOrBuffer,)| {
        match msg {
            StringOrBuffer::Buffer(dest) => {
                IsolateState::with_mut(scope, |state| {
                    if let Some(cb) = &mut state.worker_to_embedder_cb {
                        (cb)(PipedMessage::PostedBytes(dest.0))
                    }
                });
            }
            StringOrBuffer::String(dest) => {
                IsolateState::with_mut(scope, |state| {
                    if let Some(cb) = &mut state.worker_to_embedder_cb {
                        (cb)(PipedMessage::PostedString(dest))
                    }
                });
            }
        }

        return Ok(Skip {})
    });
}

fn set_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap(scope, args, retval, |scope, (cb_func,): (Option<v8::Local::<v8::Function>>,)| {
        match cb_func {
            Some(cb_func) => {
                if cb_func.is_api_wrapper() || cb_func.internal_field_count() != 0 {
                    return Err(NeptuneError::StaticTypeError("Function must not be a internal API object"));
                }

                let cb_func = v8::Global::new(scope, cb_func);

                IsolateState::with_mut(scope, |state| {
                    state.embedder_to_worker_cb = Some(cb_func);
                });
            }
            None => {
                IsolateState::with_mut(scope, |state| {
                    state.embedder_to_worker_cb = None;
                });
            }
        }

        Ok(Skip {})
    });
}

fn get_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // This function is small enough that this makes sense to be raw 
    let state = scope.get_slot::<IsolateState>().unwrap();
    if let Some(cb) = &state.embedder_to_worker_cb {
        let local_cb = v8::Local::new(scope, cb);
        retval.set(local_cb.into());
    }
}

pub struct EmbedderPipeGlobals {}
impl Globals for EmbedderPipeGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        Self::add(scope, global, "postMessage", post_message);
        Self::add(scope, global, "getMessageCallback", get_message_callback);
        Self::add(scope, global, "setMessageCallback", set_message_callback);
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![("postMessage", post_message.map_fn_to()), ("getMessageCallback", get_message_callback.map_fn_to()), ("setMessageCallback", set_message_callback.map_fn_to())]
    }
}
