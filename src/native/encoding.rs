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
            Self::ExpectedString => "Expected string".into(),
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

fn text_decode_parse_label<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>, 
    retval: v8::ReturnValue,
) {
    wrap_raw::<_, EncodingError>(scope, args, retval, |scope, args, mut retval| {
        let label = v8::Local::<v8::String>::try_from(args.get(1))
            .map_err(EncodingError::DataError)?
            .to_rust_string_lossy(scope);

        // MDN says to throw a RangeError "if the value of label is unknown, or is one of the values leading to a 'replacement' decoding algorithm ("iso-2022-cn" or "iso-2022-cn-ext")."
        let encoding = encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
            .ok_or_else(|| EncodingError::InvalidEncodingLabel(label))?;

        let encoding_str = encoding.name().to_lowercase();

        let s = v8::String::new_from_one_byte(
            scope,
            &encoding_str.as_ref(),
            v8::NewStringType::Normal,
            )
            .ok_or(EncodingError::DataInvalid)?;
        
        retval.set(s.into());

        Ok(())
    });
}

fn text_decode_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    retval: v8::ReturnValue,
) {
    wrap_raw::<_, EncodingError>(scope, args, retval, |scope, args, mut retval| {
        // args.get(0) -> Buffer (Uint8Array/ArrayBuffer)
        // args.get(1) -> Label (String)
        // args.get(2) -> fatal (Bool)
        // args.get(3) -> ignoreBOM (Bool)
        
        let label = v8::Local::<v8::String>::try_from(args.get(1))
            .map_err(EncodingError::DataError)?
            .to_rust_string_lossy(scope);
        let fatal = args.get(2).is_true();
        let ignore_bom = args.get(3).is_true();

        // fast path from deno bc im too lazy and deno has a perfectly working one
        if label == "utf-8" || label == "utf8" || label == "unicode-1-1-utf-8" {
            disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

            let buffer = ZeroCopyBuf::try_from_v8(scope, args.get(0))
            .ok_or(EncodingError::ExpectedArrayBufferOrArrayBufferView)?;

            if buffer.is_ascii() {
                // ASCII fast path. Pure ASCII inputs (the dominant real-world case for
                // HTTP/JSON bodies, file reads, etc.) are valid UTF-8 with no BOM, so we
                // can short-circuit straight to `new_from_one_byte` and skip both the
                // 3-byte BOM check and V8's internal UTF-8 validation pass.
                let s = v8::String::new_from_one_byte(
                scope,
                &buffer,
                v8::NewStringType::Normal,
                )
                .ok_or(EncodingError::BufferTooLong)?;
                retval.set(s.into());
            } else {
                let buf = if !ignore_bom
                    && buffer.len() >= 3
                    && buffer[0] == 0xef
                    && buffer[1] == 0xbb
                    && buffer[2] == 0xbf
                {
                    &buffer[3..]
                } else {
                    &buffer
                };

                if fatal {
                    // If fatal, we need to validate for utf8
                    std::str::from_utf8(buf).map_err(|_| EncodingError::DataInvalid)?;
                }

                // If `String::new_from_utf8()` returns `None`, this means that the
                // length of the decoded string would be longer than what V8 can
                // handle. In this case we return `RangeError`.
                //
                // For more details see:
                // - https://encoding.spec.whatwg.org/#dom-textdecoder-decode
                // - https://github.com/denoland/deno/issues/6649
                // - https://github.com/v8/v8/blob/d68fb4733e39525f9ff0a9222107c02c28096e2a/include/v8.h#L3277-L3278
                let text = v8::String::new_from_utf8(scope, buf, v8::NewStringType::Normal).ok_or(EncodingError::BufferTooLong)?;
                retval.set(text.into())
            }

            return Ok(());
        }

        // fall back to using encoding_rs which handles BOM stripping and replacement characters internally.
        let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
            .ok_or_else(|| EncodingError::InvalidEncodingLabel(label))?;

        let mut decoder = if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder()
        };

        {
            disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

            let buffer = ZeroCopyBuf::try_from_v8(scope, args.get(0))
            .ok_or(EncodingError::ExpectedArrayBufferOrArrayBufferView)?;

            // Calculate the maximum possible size for the output string as encoding_rs uses string capacity as limit of decoding
            let max_len = decoder.max_utf8_buffer_length(buffer.len()).unwrap_or(0);
            let mut output = String::with_capacity(max_len);

            let (result, _read, had_errors) = decoder.decode_to_string(&buffer, &mut output, true);

            if fatal && result == encoding_rs::CoderResult::InputEmpty && had_errors {
                return Err(EncodingError::DataInvalid);
            }

            let v8_string = v8::String::new_from_utf8(
                scope, 
                output.as_bytes(), 
                v8::NewStringType::Normal
            ).ok_or(EncodingError::ValueTooLarge)?;

            retval.set(v8_string.into());
        }

        Ok(())
    });
}

pub struct EncodingApiGlobals {}
impl Globals for EncodingApiGlobals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) {
        let bobj = Self::get_bootstrap(scope, global);
        Self::add(scope, bobj, "textEncodeImpl", text_encode_impl);
        Self::add(scope, bobj, "textEncodeIntoImpl", text_encode_into_impl);
        Self::add(scope, bobj, "textDecodeImpl", text_decode_impl);
        Self::add(scope, bobj, "textDecodeParseLabel", text_decode_parse_label);
    }

    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)> {
        vec![
            ("textEncodeImpl", text_encode_impl.map_fn_to()), 
            ("textEncodeIntoImpl", text_encode_into_impl.map_fn_to()),
            ("textDecodeImpl", text_decode_impl.map_fn_to()),
            ("textDecodeParseLabel", text_decode_parse_label.map_fn_to()),
        ]
    }

    fn js_files() -> Vec<(Cow<'static, str>, Cow<'static, str>)> {
        vec![
            (Cow::Borrowed("neptune/encoder/encoderapi.js"), Cow::Borrowed(include_str!("../js/neptune/encoder/encoderapi.js")))
        ]
    }
}
