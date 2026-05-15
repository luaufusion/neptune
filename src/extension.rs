use std::borrow::Cow;
use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};

use v8::disallow_javascript_execution_scope;

use crate::buffer::{CopiedBuffer, ZeroCopyBuf};

pub enum NeptuneError {
    StaticTypeError(&'static str),
    TypeError(String),
    DataError(v8::DataError),
    ExpectedArrayBufferOrArrayBufferView,
    ExpectedString,
    ExpectedArrayBufferOrArrayBufferViewOrString,

    // encoding
    Base64Decode,
    InvalidEncodingLabel(String),
    BufferTooLong,
    ValueTooLarge,
    BufferTooSmall,
    DataInvalid,
}

impl std::fmt::Display for NeptuneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaticTypeError(s) => f.write_str(s),
            Self::TypeError(o) => f.write_str(&o),
            Self::DataError(d) => f.write_str(&d.to_string()),
            Self::ExpectedArrayBufferOrArrayBufferView => f.write_str("Expected ArrayBuffer or ArrayBufferView"),
            Self::ExpectedArrayBufferOrArrayBufferViewOrString => f.write_str("Expected either a string, ArrayBuffer or ArrayBufferView"),
            Self::ExpectedString => f.write_str("Expected string"),

            // encoding
            Self::Base64Decode => f.write_str("Failed to decode base64"),
            Self::InvalidEncodingLabel(s) => f.write_str(&format!("The encoding label provided ('{s}') is invalid.")),
            Self::BufferTooLong => f.write_str("buffer exceeds maximum length"),
            Self::ValueTooLarge => f.write_str("Value too large to decode"),
            Self::BufferTooSmall => f.write_str("Provided buffer too small"),
            Self::DataInvalid => f.write_str("The encoded data is not valid"),
        }
    }
}

pub enum StringOrBuffer {
    String(String),
    Buffer(CopiedBuffer)
}

pub struct Skip {}

/// Trait to convert from v8 to the type
/// 
/// Supported props (besides standard conversion from v8::Local's, Option<T>):
/// - String
/// - bool
/// - i32
/// - f64
/// - CopiedBuffer
/// - StringOrBuffer
/// - Skip
pub trait FromV8<'s>: Sized {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError>;
}

impl<'s, T> FromV8<'s> for v8::Local<'s, T> where v8::Local<'s, T>: TryFrom<v8::Local<'s, v8::Value>, Error = v8::DataError> {
    fn from_v8(_scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        match v8::Local::<T>::try_from(value) {
            Ok(v) => Ok(v),
            Err(e) => Err(NeptuneError::DataError(e))
        }
    }
}

impl<'s, T: FromV8<'s>> FromV8<'s> for Option<T> {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        if value.is_null_or_undefined() {
            Ok(None)
        } else {
            T::from_v8(scope, value).map(Some)
        }
    }
}

impl<'s> FromV8<'s> for StringOrBuffer {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        match String::from_v8(scope, value) {
            Ok(s) => return Ok(StringOrBuffer::String(s)),
            Err(NeptuneError::ExpectedString) => {
                return Ok(StringOrBuffer::Buffer(CopiedBuffer::from_v8(scope, value).map_err(|_| NeptuneError::ExpectedArrayBufferOrArrayBufferViewOrString)?))
            },
            Err(e) => return Err(e)
        }
    }
}

impl<'s> FromV8<'s> for Skip {
    fn from_v8(_scope: &mut v8::PinScope<'s, '_>, _value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        Ok(Skip {})
    }
}

impl<'s> FromV8<'s> for String {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        if !value.is_string() {
            return Err(NeptuneError::ExpectedString)
        }
        Ok(value.to_rust_string_lossy(scope))
    }
}

impl<'s> FromV8<'s> for bool {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        if !value.is_boolean() {
            return Err(NeptuneError::StaticTypeError("Expected boolean for argument"))
        }
        Ok(value.boolean_value(scope))
    }
}

impl<'s> FromV8<'s> for i32 {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        if !value.is_int32() && !value.is_number() {
            return Err(NeptuneError::StaticTypeError("Expected integer for argument"))
        }
        value.int32_value(scope).ok_or(NeptuneError::StaticTypeError("Failed to convert i32 to number"))
    }
}

impl<'s> FromV8<'s> for f64 {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        if !value.is_number() {
            return Err(NeptuneError::StaticTypeError("Expected number for argument"))
        }
        value.number_value(scope).ok_or(NeptuneError::StaticTypeError("Failed to convert f64 to number"))
    }
}

impl<'s> FromV8<'s> for CopiedBuffer {
    fn from_v8(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        let buffer = CopiedBuffer::try_from_v8(scope, value).ok_or(NeptuneError::ExpectedArrayBufferOrArrayBufferView)?;
        Ok(buffer)
    }
}

/// Trait to convert from value into the type
/// 
/// Supported props (besides standard conversion from v8::Local's, Option<T>):
/// - String
/// - bool
/// - i32
/// - f64
/// - Skip
pub trait IntoV8<'s>: Sized {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError>;
}

impl<'s, T> IntoV8<'s> for v8::Local<'s, T> 
where 
    v8::Local<'s, T>: Into<v8::Local<'s, v8::Value>> 
{
    fn into_v8(self, _scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        Ok(self.into())
    }
}

impl<'s, T: IntoV8<'s>> IntoV8<'s> for Option<T> {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        match self {
            Some(s) => s.into_v8(scope),
            None => Ok(v8::null(scope).into())
        }
    }
}

impl<'s> IntoV8<'s> for String {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        let s = v8::String::new(scope, &self).ok_or(NeptuneError::StaticTypeError("Failed to allocate string"))?;
        Ok(s.into())
    }
}

impl<'s> IntoV8<'s> for bool {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        let s = v8::Boolean::new(scope, self);
        Ok(s.into())
    }
}

impl<'s> IntoV8<'s> for i32 {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        let s = v8::Integer::new(scope, self);
        Ok(s.into())
    }
}

impl<'s> IntoV8<'s> for f64 {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        let s = v8::Number::new(scope, self);
        Ok(s.into())
    }
}

impl<'s> IntoV8<'s> for Skip {
    fn into_v8(self, scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, NeptuneError> {
        Ok(v8::undefined(scope).into())
    }
}

/// Trait to convert function arguments to arguments
pub trait FromV8FunctionCallbackArguments<'s>: Sized {
    fn from_v8_fargs(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        start_index: usize,
    ) -> Result<Self, NeptuneError>;
}

macro_rules! impl_from_v8_fargs {
  ($($t:ident),*) => {
    impl<'s, $($t),*> FromV8FunctionCallbackArguments<'s> for ($($t,)*)
    where
      $($t: FromV8<'s>),*
    {
      #[allow(unused_assignments)]
      fn from_v8_fargs(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        start_index: usize,
      ) -> Result<Self, NeptuneError> {
        let mut i = start_index;
        Ok(($(
          {
            let res = $t::from_v8(scope, args.get(i as i32))?;
            i += 1;
            res
          },
        )*))
      }
    }
  };
}

impl_from_v8_fargs!(A);
impl_from_v8_fargs!(A, B);
impl_from_v8_fargs!(A, B, C);
impl_from_v8_fargs!(A, B, C, D);
impl_from_v8_fargs!(A, B, C, D, E);
impl_from_v8_fargs!(A, B, C, D, E, F);
impl_from_v8_fargs!(A, B, C, D, E, F, G);
impl_from_v8_fargs!(A, B, C, D, E, F, G, H);

/// Trait to convert from v8 to the type for cases that require javascript to be disabled (ZeroCopyBuf's etc)
/// 
/// Supported props (besides standard conversion for FromV8's):
/// - ZeroCopyBuf
pub trait FromV8NonReentrant<'s>: Sized {
    fn from_v8_non_reentrant(scope:  &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError>;
}

impl<'s, T: FromV8<'s>> FromV8NonReentrant<'s> for T {
    fn from_v8_non_reentrant(scope:  &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        T::from_v8(scope, value)
    }
}

impl<'s> FromV8NonReentrant<'s> for ZeroCopyBuf<'s> {
    fn from_v8_non_reentrant(scope: &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>, value: v8::Local<'s, v8::Value>) -> Result<Self, NeptuneError> {
        let buffer = ZeroCopyBuf::try_from_v8(scope, value).ok_or(NeptuneError::ExpectedArrayBufferOrArrayBufferView)?;
        Ok(buffer)
    }
}

/// Trait to convert function arguments to arguments
pub trait FromV8NonReentrantFunctionCallbackArguments<'s>: Sized {
    fn from_v8_fargs_nonreentant(
        scope: &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>,
        args: &v8::FunctionCallbackArguments<'s>,
        start_index: usize,
    ) -> Result<Self, NeptuneError>;
}

macro_rules! impl_from_v8_fargs_nr {
  ($($t:ident),*) => {
    impl<'s, $($t),*> FromV8NonReentrantFunctionCallbackArguments<'s> for ($($t,)*)
    where
      $($t: FromV8NonReentrant<'s>),*
    {
      #[allow(unused_assignments)]
      fn from_v8_fargs_nonreentant(
        scope: &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>,
        args: &v8::FunctionCallbackArguments<'s>,
        start_index: usize,
      ) -> Result<Self, NeptuneError> {
        let mut i = start_index;
        Ok(($(
          {
            let res = $t::from_v8_non_reentrant(scope, args.get(i as i32))?;
            i += 1;
            res
          },
        )*))
      }
    }
  };
}

impl_from_v8_fargs_nr!(A);
impl_from_v8_fargs_nr!(A, B);
impl_from_v8_fargs_nr!(A, B, C);
impl_from_v8_fargs_nr!(A, B, C, D);
impl_from_v8_fargs_nr!(A, B, C, D, E);
impl_from_v8_fargs_nr!(A, B, C, D, E, F);
impl_from_v8_fargs_nr!(A, B, C, D, E, F, G);
impl_from_v8_fargs_nr!(A, B, C, D, E, F, G, H);


/// Helper method to wrap a function with automatic argument handling
pub fn wrap<'s, Func, FuncArgs, FuncRet>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    retval: v8::ReturnValue,
    func: Func, 
) 
    where Func: FnOnce(&mut v8::PinScope<'s, '_>, FuncArgs) -> Result<FuncRet, NeptuneError>,
    FuncArgs: FromV8FunctionCallbackArguments<'s>,
    FuncRet: IntoV8<'s>,
{
    wrap_raw(scope, args, retval, |scope, args, mut retval| {
        let args = FuncArgs::from_v8_fargs(scope, &args, 0)?;
        let ret = func(scope, args)?;
        let rv = ret.into_v8(scope)?;
        retval.set(rv);
        Ok::<_, NeptuneError>(())
    })
}

/// Helper method to wrap a function with automatic argument handling (non-reentrant case)
/// 
/// The scope passed will be a DisallowJavascriptExecutionScope to enforce the no-JS invariant
pub fn wrap_nonreentrant<'s, Func, FuncArgs, FuncRet>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    retval: v8::ReturnValue,
    func: Func, 
) 
    where Func: FnOnce(&mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>, FuncArgs) -> Result<FuncRet, NeptuneError>,
    FuncArgs: FromV8NonReentrantFunctionCallbackArguments<'s>,
    FuncRet: IntoV8<'s>,
{
    wrap_raw(scope, args, retval, |scope, args, mut retval| {
        let ret = {
            disallow_javascript_execution_scope!(let djs_scope, scope,v8::OnFailure::ThrowOnFailure);
            let args = FuncArgs::from_v8_fargs_nonreentant(djs_scope, &args, 0)?;
            func(djs_scope, args)?
        };
        let rv = ret.into_v8(scope)?;
        retval.set(rv);
        Ok::<_, NeptuneError>(())
    })
}

/// Helper method to wrap a function rawly 
#[inline(always)]
pub fn wrap_raw<'s, Func, E>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    retval: v8::ReturnValue,
    func: Func, 
) 
    where Func: FnOnce(&mut v8::PinScope<'s, '_>, v8::FunctionCallbackArguments<'s>, v8::ReturnValue) -> Result<(), E>,
    E: Display
{
    let res = catch_unwind(AssertUnwindSafe(|| func(scope, args, retval)));
    match res {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            let Some(msg) = v8::String::new(scope, &e.to_string()) else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
        Err(p) => {
            let err_msg = {
                // If downcastable to String, use it
                if let Some(s) = p.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = p.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    // Otherwise, use the debug representation
                    format!("Panic occurred in callback: {:?}", p)
                }
            };

            std::mem::forget(catch_unwind(AssertUnwindSafe(move || drop(p))));

            let Some(msg) = v8::String::new(scope, &err_msg) else {
                return;
            };
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    }
}

/// Defines a set of globals
pub trait Globals {
    fn register<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>);

    /// Any external references needed for snapshotting
    fn get_external_references() -> Vec<(&'static str, v8::FunctionCallback)>;

    /// Adds a global to global scope
    /// 
    /// Should not be overriden
    fn add<'s, F>(
        scope: &mut v8::PinScope<'s, '_>, 
        global: v8::Local<v8::Object>,
        name: &str, 
        callback: F 
    ) 
    where
        F: v8::MapFnTo<v8::FunctionCallback> 
    {
        let name = v8::String::new(scope, name).unwrap();
        let tmpl = v8::FunctionTemplate::new(scope, callback);
        let val = tmpl.get_function(scope).unwrap();
        val.set_name(name.into());
        global.set(scope, name.into(), val.into());
    }

    /// Gets the bootstrap object
    fn get_bootstrap<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<v8::Object>) -> v8::Local<'s, v8::Object> {
        let name = v8::String::new(scope, "bootstrap").unwrap();
        let bobj = global.get(scope, name.into()).unwrap();
        v8::Local::<v8::Object>::try_from(bobj).unwrap()

    }   

    /// What js files to load (if any)
    fn js_files() -> Vec<(Cow<'static, str>, Cow<'static, str>)> {
        vec![]
    }
}