// structuredClone is inspired by Deno
use v8::{MapFnTo, ValueDeserializerHelper, ValueSerializerHelper};

use crate::extension::Globals;

struct SerializeDeserialize {}

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
        false
    }

    fn is_host_object<'s, 'i>(
        &self,
        _scope: &mut v8::PinScope<'s, 'i>,
        _object: v8::Local<'s, v8::Object>,
    ) -> Option<bool> {
        Some(false)
    }

    fn write_host_object<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i>,
        _object: v8::Local<'s, v8::Object>,
        _value_serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        let message = v8::String::new(scope, "Unsupported object type").unwrap();
        self.throw_data_clone_error(scope, message);
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
        _scope: &mut v8::PinScope<'s, 'i>,
        _value_deserializer: &dyn v8::ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
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

pub struct StructuredCloneGlobals {}
impl Globals for StructuredCloneGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        Self::add(scope, global, "structuredClone", native_structured_clone);
    }

    fn get_external_references() -> Vec<v8::FunctionCallback> {
        vec![native_structured_clone.map_fn_to()]
    }
}
