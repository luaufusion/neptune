// structuredClone is inspired by Deno
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

use crate::state_ref;

pub(crate) const CLONE_REGISTRY: &[(&str, u32)] = &[
    ("DOMException", 1),
    ("QuotaExceededError", 2),
];

struct SerializeDeserialize {

}

impl v8::ValueSerializerImpl for SerializeDeserialize {
    fn throw_data_clone_error<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
    }

    fn get_shared_array_buffer_id<'s, 'i>(
        &self,
        _scope: &mut v8::PinScope<'s, 'i>,
        _shared_array_buffer: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        None
    }

    fn get_wasm_module_transfer_id<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        _module: v8::Local<v8::WasmModuleObject>,
    ) -> Option<u32> {
        let message = v8::String::new(scope, "Wasm modules cannot be stored")?;
        self.throw_data_clone_error(scope, message);
        return None;
    }

    fn has_custom_host_object(&self, _isolate: &v8::Isolate) -> bool {
        true
    }

    fn is_host_object<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<bool> {
        let brand_name = v8::String::new(scope, "NeptuneClone")?;
        let private_key = v8::Private::for_api(scope, Some(brand_name));
        object.has_private(scope, private_key)
    }

    fn write_host_object<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        object: v8::Local<'s, v8::Object>,
        serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        let context = scope.get_current_context();

        let brand_name = v8::String::new(scope, "NeptuneClone")?;
        let private_key = v8::Private::for_api(scope, Some(brand_name));

        if let Some(tag_val) = object.get_private(scope, private_key) {
            if let Ok(tag_uint) = v8::Local::<v8::Uint32>::try_from(tag_val) {
                let tag = tag_uint.value();

                match tag {
                    1 | 2 => {
                        // DOMException or QuotaExceededObject
                        let msg_key = v8::String::new(scope, "message").unwrap();
                        let name_key = v8::String::new(scope, "name").unwrap();
                        let empty_str = v8::String::empty(scope);

                        let msg_val = object.get(scope, msg_key.into()).unwrap_or(empty_str.into());
                        let name_val = object.get(scope, name_key.into()).unwrap_or(empty_str.into());

                        // Write to serializer
                        serializer.write_uint32(tag);
                        serializer.write_value(context, msg_val)?;
                        serializer.write_value(context, name_val)?;

                        return Some(true);
                    },
                    _ => return None,
                }
            }
        }

        let err_msg = v8::String::new(scope, "DataCloneError").unwrap();
        self.throw_data_clone_error(scope, err_msg);
        None
    }
}

impl v8::ValueDeserializerImpl for SerializeDeserialize {
    fn get_shared_array_buffer_from_id<'s, 'i>(
        &self,
        _scope: &mut v8::PinScope<'s, 'i>,
        _transfer_id: u32,
    ) -> Option<v8::Local<'s, v8::SharedArrayBuffer>> {
        None
    }

    fn get_wasm_module_from_id<'s, 'i>(
        &self,
        _scope: &mut v8::PinScope<'s, 'i>,
        _clone_id: u32,
    ) -> Option<v8::Local<'s, v8::WasmModuleObject>> {
        None
    }

    fn read_host_object<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        deserializer: &dyn v8::ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let context = scope.get_current_context();

        let mut tag = 0;
        deserializer.read_uint32(&mut tag);

        if tag == 1 || tag == 2 { // DOMException or QuotaExceededError
            let msg_val = deserializer.read_value(context)?;
            let name_val = deserializer.read_value(context)?;

            v8::allow_javascript_execution_scope!(let scope, scope);
            state_ref!(let state, scope);
            let ctor = state.cloneables.get(&tag)?;
            let ctor = v8::Local::new(scope, ctor);

            // (Executing JS here is perfectly safe because the GC isn't
            // mid-traversal; we are at the end of the deserialization pipeline).
            let cloned_exception = ctor.new_instance(scope, &[msg_val, name_val])?;

            return Some(cloned_exception);
        }

        None
    }
}

// Use V8's internal serialization to perform a deep copy
fn native_structured_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let value = args.get(0);

    let value_serializer = v8::ValueSerializer::new(scope, Box::new(SerializeDeserialize {}));
    value_serializer.write_header();

    v8::tc_scope!(let scope, scope);

    let ret = value_serializer.write_value(scope.get_current_context(), value);
    if scope.has_caught() || scope.has_terminated() {
        scope.rethrow();
        let v = v8::undefined(scope);
        rv.set(v.into());
        return;
    }

    if ret != Some(true) {
        let Some(msg) = v8::String::new(scope, "Failed to serialize response") else {
            return;
        };
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    }

    let data = value_serializer.release();

    let value_deserializer = v8::ValueDeserializer::new(scope, Box::new(SerializeDeserialize {}), &data);
    let parsed_header = value_deserializer.read_header(scope.get_current_context())
    .unwrap_or_default();
    if !parsed_header {
        let Some(msg) = v8::String::new(scope, "could not deserialize value") else {
            return;
        };
        let error = v8::Exception::range_error(scope, msg);
        scope.throw_exception(error);
        return;
    }

    let value = value_deserializer.read_value(scope.get_current_context());
    match value {
        Some(deserialized) => {
            rv.set(deserialized);
        },
        None => {
            let Some(msg) = v8::String::new(scope, "could not deserialize value") else {
                return;
            };
            let error = v8::Exception::range_error(scope, msg);
            scope.throw_exception(error);
            return;
        },
    }
    return;
}

neptune_macros::define_globals! {
    pub struct StructuredCloneGlobals {
        structuredClone: native_structured_clone,
    }
}
