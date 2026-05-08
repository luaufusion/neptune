use tokio::sync::{Mutex, mpsc::{UnboundedReceiver, UnboundedSender}};
use v8::{cppgc::GarbageCollected};

use crate::{extension::{MethodBuilder, NativeObject, NativeObjectBuilder}, runtime::Value};

pub enum StreamedData {
    Bytes(Vec<u8>),
    String(String)
}

/// Bridge object for sending and receiving bytes
pub struct Stream {
    pub(super) tx: UnboundedSender<StreamedData>,
    pub(super) rx: Mutex<UnboundedReceiver<StreamedData>>,
}

impl Stream {
    /// Creates a new Stream object
    pub fn new(tx: UnboundedSender<StreamedData>, rx: UnboundedReceiver<StreamedData>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx)
        }
    }
}

unsafe impl GarbageCollected for Stream {
    fn get_name(&self) -> &'static std::ffi::CStr {
        c"Stream"
    }

    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {
        // No fields to trace
    }
}

impl NativeObject for Stream {
    fn class_name() -> &'static str {
        "Stream"
    }

    fn bind_methods<'a, 's, 'i>(builder: NativeObjectBuilder) {
        builder
        .method("send", stream_send)
        .method("recv", stream_recv);
    }
}

fn stream_send<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    MethodBuilder::bind_sync_method::<Stream, _>(
        scope,
        args,
        retval,
        |bridge, scope, args| {
            let arg0 = args.get(0);
            
            if arg0.is_array_buffer_view() {
                let view = v8::Local::<v8::ArrayBufferView>::try_from(arg0).unwrap();
                let ab = view.buffer(scope).expect("View has no buffer");
                let store = ab.get_backing_store();
                
                let offset = view.byte_offset();
                let len = view.byte_length();
                
                // Slice the backing store using the view's offset and length
                let bytes = &store[offset..offset + len];
                let dest: Vec<u8> = bytes.iter().map(|c| c.get()).collect();

                bridge.tx.send(StreamedData::Bytes(dest))?;
            } else if arg0.is_array_buffer() {
                let ab = v8::Local::<v8::ArrayBuffer>::try_from(arg0).unwrap();
                let dest: Vec<u8> = ab.get_backing_store().iter().map(|c| c.get()).collect();
                bridge.tx.send(StreamedData::Bytes(dest))?;
            } else if arg0.is_string() {
                let s= v8::Local::<v8::String>::try_from(arg0).unwrap();
                let dest = s.to_rust_string_lossy(scope);
                bridge.tx.send(StreamedData::String(dest))?;
            } else {
                return Err("Either ArrayBufferView/ArrayBuffer/String expected".into());
            };

            Ok(Value::Undefined)
        },
    );
}

fn stream_recv<'s>(
    scope: &mut v8::PinScope<'s, '_>, 
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    MethodBuilder::bind_async_method::<Stream, _, _>(
        scope,
        args,
        retval,
        |bridge, _scope, _args| {
            Ok(async move {
                let mut rx = bridge.rx.lock().await;
                match rx.recv().await {
                    Some(v) => {
                        match v {
                            StreamedData::Bytes(b) => Ok(Value::Buffer(b)),
                            StreamedData::String(s) => Ok(Value::String(s))
                        }
                    }
                    None => Ok(Value::Null)
                }
            })
        },
    );
}