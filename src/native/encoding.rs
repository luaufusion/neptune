use std::{borrow::Cow, fmt::Display};

use v8::{MapFnTo, OnFailure, disallow_javascript_execution_scope};

use crate::{buffer::ZeroCopyBuf, extension::{Globals, wrap_raw}};

#[allow(unused)]
enum EncodingError {
    Base64Decode,
    InvalidEncodingLabel(String),
    BufferTooLong,
    ValueTooLarge,
    BufferTooSmall,
    DataInvalid,
    ExpectedArrayBufferOrArrayBufferView,
    ExpectedString,
    DataError(v8::DataError),
}

impl EncodingError {
    fn display(&self) -> Cow<'_, str> {
        match self {
            Self::Base64Decode => "Failed to decode base64".into(),
            Self::InvalidEncodingLabel(s) => format!("The encoding label provided ('{s}') is invalid.").into(),
            Self::BufferTooLong => "buffer exceeds maximum length".into(),
            Self::ValueTooLarge => "Value too large to decode".into(),
            Self::BufferTooSmall => "Provided buffer too small".into(),
            Self::DataInvalid => "The encoded data is not valid".into(),
            Self::DataError(s) => s.to_string().into(),
            Self::ExpectedArrayBufferOrArrayBufferView => "Expected ArrayBuffer or ArrayBufferView".into(),
            Self::ExpectedString => "Expected string".into()
        }
    }
}

impl Display for EncodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

fn text_encode_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap_raw::<_, EncodingError>(scope, args, retval, |scope, args, mut retval| {
        let text = v8::Local::<v8::String>::try_from(args.get(0)).map_err(EncodingError::DataError)?;
        let byte_len = text.utf8_length(scope);

        // encode needs to create the arraybuffer
        let new_ab = v8::ArrayBuffer::new(scope, byte_len);

        if byte_len > 0 {
            disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

            let mut buffer = ZeroCopyBuf::try_from_v8(scope, new_ab.into()).ok_or(EncodingError::ExpectedArrayBufferOrArrayBufferView)?;

            text.write_utf8_v2(
                scope,
                &mut buffer,
                v8::WriteFlags::kReplaceInvalidUtf8,
                None,
            );
        }

        let ui8 = v8::Uint8Array::new(scope, new_ab, 0, byte_len).unwrap();

        retval.set(ui8.into());

        Ok(())
    });
}

fn text_encode_into_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap_raw::<_, EncodingError>(scope, args, retval, |scope, args, mut retval| {
        let text = v8::Local::<v8::String>::try_from(args.get(0)).map_err(EncodingError::DataError)?;

        let (chars_read, bytes_written) = {
            disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

            let mut buffer = ZeroCopyBuf::try_from_v8(scope, args.get(1)).ok_or(EncodingError::ExpectedArrayBufferOrArrayBufferView)?;

            let mut chars_read = 0;

            let bytes_written = text.write_utf8_v2(
                scope,
                &mut buffer,
                v8::WriteFlags::kReplaceInvalidUtf8,
                Some(&mut chars_read),
            );

            (chars_read, bytes_written)
        };

        // Return [read, written]
        let arr = v8::Array::new(scope, 2);
        
        let read_val = v8::Integer::new(scope, chars_read as i32);
        let written_val = v8::Integer::new(scope, bytes_written as i32);
        
        arr.set_index(scope, 0, read_val.into());
        arr.set_index(scope, 1, written_val.into());

        retval.set(arr.into());

        Ok(())
    });
}

pub struct EncodingApiGlobals {}
impl Globals for EncodingApiGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        let bobj = Self::get_bootstrap(scope, global);
        Self::add(scope, bobj, "textEncodeImpl", text_encode_impl);
        Self::add(scope, bobj, "textEncodeIntoImpl", text_encode_into_impl);
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![("textEncodeImpl", text_encode_impl.map_fn_to()), ("textEncodeIntoImpl", text_encode_into_impl.map_fn_to())]
    }

    fn js_files() -> Vec<(Cow<'static, str>, Cow<'static, str>)> {
        vec![
            (Cow::Borrowed("neptune/encoder/encoderapi.js"), Cow::Borrowed(include_str!("../js/neptune/encoder/encoderapi.js")))
        ]
    }
}
