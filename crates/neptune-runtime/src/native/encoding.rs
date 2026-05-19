use neptune_macros::op;
use v8::{OnFailure, disallow_javascript_execution_scope};

use crate::{buffer::ZeroCopyBuf, extension::{NeptuneError, ByteString}};

#[op]
fn btoa<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    data: String,
) -> Result<String, NeptuneError> {
    for c in data.chars() {
        if c as u32 > 255 {
            return Err(NeptuneError::StaticTypeError("The string to be encoded contains characters outside of the Latin1 range."));
        }
    }
    Ok(base64_simd::STANDARD.encode_to_string(&data))
}

#[op]
fn atob<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    data: String,
) -> Result<ByteString, NeptuneError> {
    // forgiving_decode_to_vec handles whitespace stripping inherently and matches Web API spec
    let decoded = base64_simd::forgiving_decode_to_vec(data.as_bytes())
        .map_err(|_| NeptuneError::Base64Decode)?;

    Ok(ByteString(decoded))
}

#[op(raw)]
fn text_encode_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) -> Result<(), NeptuneError> {
    // NOTE: This uses a manual impl for performance
    let text = v8::Local::<v8::String>::try_from(args.get(0)).map_err(NeptuneError::DataError)?;
    let byte_len = text.utf8_length(scope);

    // encode needs to create the arraybuffer
    let new_ab = v8::ArrayBuffer::new(scope, byte_len);

    if byte_len > 0 {
        disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

        let mut buffer = ZeroCopyBuf::try_from_v8(scope, new_ab.into()).ok_or(NeptuneError::ExpectedArrayBufferOrArrayBufferView)?;

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
}

#[op(raw)]
fn text_encode_into_impl<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>, 
    mut retval: v8::ReturnValue,
) -> Result<(), NeptuneError> {
    let text = v8::Local::<v8::String>::try_from(args.get(0)).map_err(NeptuneError::DataError)?;

    let (chars_read, bytes_written) = {
        disallow_javascript_execution_scope!(let scope, scope, OnFailure::ThrowOnFailure);

        let mut buffer = ZeroCopyBuf::try_from_v8(scope, args.get(1)).ok_or(NeptuneError::ExpectedArrayBufferOrArrayBufferView)?;

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
}

#[op]
fn text_decode_parse_label<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    label: String,
) -> Result<v8::Local<'s, v8::String>, NeptuneError> {
    // MDN says to throw a RangeError "if the value of label is unknown, or is one of the values leading to a 'replacement' decoding algorithm ("iso-2022-cn" or "iso-2022-cn-ext")."
    let encoding = encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
        .ok_or_else(|| NeptuneError::InvalidEncodingLabel(label))?;

    let encoding_str = encoding.name().to_lowercase();

    let s = v8::String::new_from_one_byte(
        scope,
        &encoding_str.as_ref(),
        v8::NewStringType::Normal,
        )
        .ok_or(NeptuneError::DataInvalid)?;
    
    Ok(s)
}

#[op(nonreentrant)]
fn text_decode_impl<'s>(
    scope: &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>,
    buffer: ZeroCopyBuf, 
    label: String, 
    fatal: bool, 
    ignore_bom: bool
) -> Result<v8::Local<'s, v8::String>, NeptuneError> {
    // args.get(0) -> Buffer (Uint8Array/ArrayBuffer)
    // args.get(1) -> Label (String)
    // args.get(2) -> fatal (Bool)
    // args.get(3) -> ignoreBOM (Bool)
    
    // fast path from deno bc im too lazy and deno has a perfectly working one
    if label == "utf-8" || label == "utf8" || label == "unicode-1-1-utf-8" {
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
            .ok_or(NeptuneError::BufferTooLong)?;
            return Ok(s.into());
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
                std::str::from_utf8(buf).map_err(|_| NeptuneError::DataInvalid)?;
            }

            // If `String::new_from_utf8()` returns `None`, this means that the
            // length of the decoded string would be longer than what V8 can
            // handle. In this case we return `RangeError`.
            //
            // For more details see:
            // - https://encoding.spec.whatwg.org/#dom-textdecoder-decode
            // - https://github.com/denoland/deno/issues/6649
            // - https://github.com/v8/v8/blob/d68fb4733e39525f9ff0a9222107c02c28096e2a/include/v8.h#L3277-L3278
            let text = v8::String::new_from_utf8(scope, buf, v8::NewStringType::Normal).ok_or(NeptuneError::BufferTooLong)?;
            return Ok(text.into())
        }
    }

    // fall back to using encoding_rs which handles BOM stripping and replacement characters internally.
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| NeptuneError::InvalidEncodingLabel(label))?;

    let mut decoder = if ignore_bom {
        encoding.new_decoder_without_bom_handling()
    } else {
        encoding.new_decoder()
    };

    // Calculate the maximum possible size for the output string as encoding_rs uses string capacity as limit of decoding
    let max_len = decoder.max_utf8_buffer_length(buffer.len()).unwrap_or(0);
    let mut output = String::with_capacity(max_len);

    let (result, _read, had_errors) = decoder.decode_to_string(&buffer, &mut output, true);

    if fatal && result == encoding_rs::CoderResult::InputEmpty && had_errors {
        return Err(NeptuneError::DataInvalid);
    }

    let v8_string = v8::String::new_from_utf8(
        scope, 
        output.as_bytes(), 
        v8::NewStringType::Normal
    ).ok_or(NeptuneError::ValueTooLarge)?;

    Ok(v8_string)
}

neptune_macros::define_globals! {
    pub struct EncodingApiGlobals {
        bootstrap: true,
        js_files: {
            "neptune/encoder/encoderapi.js" => include_str!("../js/neptune/encoder/encoderapi.js"),
        },
        textEncodeImpl: text_encode_impl,
        textEncodeIntoImpl: text_encode_into_impl,
        textDecodeImpl: text_decode_impl,
        textDecodeParseLabel: text_decode_parse_label,
        btoa: btoa,
        atob: atob,
    }
}
