use neptune_macros::op;
use crate::{extension::{NeptuneError, StringOrBuffer}, runtime::PipedMessage, state::IsolateState};

#[op]
fn post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    msg: StringOrBuffer,
) -> Result<(), NeptuneError> {
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

    return Ok(())
}

#[op]
fn set_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    cb_func: Option<v8::Local::<v8::Function>>,
) -> Result<(), NeptuneError> {
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

    Ok(())
}

#[op]
fn get_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<Option<v8::Local<'s, v8::Function>>, NeptuneError> {
    let state = scope.get_slot::<IsolateState>().unwrap();
    if let Some(cb) = &state.embedder_to_worker_cb {
        let local_cb = v8::Local::new(scope, cb);
        Ok(Some(local_cb))
    } else {
        Ok(None)
    }
}

neptune_macros::define_globals! {
    pub struct EmbedderPipeGlobals {
        postMessage: post_message,
        getMessageCallback: get_message_callback,
        setMessageCallback: set_message_callback,
    }
}
