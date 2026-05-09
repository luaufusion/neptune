// Copyright 2018-2026 the Deno authors. MIT license.

use std::any::TypeId;
pub use v8::cppgc::GarbageCollected;
use crate::state::IsolateState;

const CPPGC_SINGLE_TAG: u16 = 1;

#[repr(C)]
struct CppGcObject<T: GarbageCollected> {
  tag: TypeId,
  member: T,
}

unsafe impl<T: GarbageCollected> v8::cppgc::GarbageCollected
  for CppGcObject<T>
{
  fn trace(&self, visitor: &mut v8::cppgc::Visitor) {
    self.member.trace(visitor);
  }

  fn get_name(&self) -> &'static std::ffi::CStr {
    self.member.get_name()
  }
}

#[doc(hidden)]
pub fn make_cppgc_empty_object<'a, 'i, T: GarbageCollected + 'static>(
  scope: &v8::PinScope<'a, 'i>,
) -> v8::Local<'a, v8::Object> {
    // Identify the exact Rust struct we are trying to wrap
    let type_id = TypeId::of::<T>();

    // Safely borrow the state to extract the template handles.
    // We do this inside a block so the `state` reference is dropped 
    // BEFORE we touch `scope` again to avoid Rust borrow checker errors.
    let (specific_tpl_opt, fallback_tpl) = {
        let state = scope.get_slot::<IsolateState>().expect("IsolateState is missing");
        
        let specific = state.cppgc_type_templates.get(&type_id).cloned();
        let fallback = state.cppgc_fallback_template.clone();
        
        (specific, fallback)
    };

    // Create the object based on whether we had a specific template
    match specific_tpl_opt {
        Some(global_function_tpl) => {
            let function_tpl = v8::Local::new(scope, global_function_tpl);
            
            let instance_tpl = function_tpl.instance_template(scope);

            instance_tpl.new_instance(scope).unwrap()
        }
        None => {
            // Path B: Fallback (Just an opaque C++ struct passed to JS)
            let fallback_obj_tpl = v8::Local::new(scope, fallback_tpl);
            
            // Because our fallback is an ObjectTemplate, we can instantiate it directly
            fallback_obj_tpl.new_instance(scope).unwrap()
        }
    }
}

pub fn make_cppgc_object<'a, 'i, T: GarbageCollected + 'static>(
  scope: &mut v8::PinScope<'a, 'i>,
  t: T,
) -> v8::Local<'a, v8::Object> {
  let obj = make_cppgc_empty_object::<T>(scope);
  wrap_object(scope, obj, t)
}

// Wrap an API object (eg: `args.This()`)
pub fn wrap_object<'a, T: GarbageCollected + 'static>(
  isolate: &mut v8::Isolate,
  obj: v8::Local<'a, v8::Object>,
  t: T,
) -> v8::Local<'a, v8::Object> {
  let heap = isolate.get_cpp_heap().unwrap();
  unsafe {
    let member = v8::cppgc::make_garbage_collected(
      heap,
      CppGcObject {
        tag: TypeId::of::<T>(),
        member: t,
      },
    );

    v8::Object::wrap::<CPPGC_SINGLE_TAG, CppGcObject<T>>(isolate, obj, &member);

    obj
  }
}

pub fn make_cppgc_proto_object<'a, 'i, T: GarbageCollected + 'static>(
  scope: &mut v8::PinScope<'a, 'i>,
  t: T,
) -> v8::Local<'a, v8::Object> {
  make_cppgc_object(scope, t)
}

pub struct UnsafePtr<T: GarbageCollected> {
  inner: v8::cppgc::UnsafePtr<CppGcObject<T>>,
  root: Option<v8::cppgc::Persistent<CppGcObject<T>>>,
}

impl<T: GarbageCollected> UnsafePtr<T> {
  #[allow(clippy::missing_safety_doc, reason = "internal hidden API")]
  pub unsafe fn as_ref(&self) -> &T {
    unsafe { &self.inner.as_ref().member }
  }
}

#[doc(hidden)]
impl<T: GarbageCollected> UnsafePtr<T> {
  /// If this pointer is used in an async function, it could leave the stack,
  /// so this method can be called to root it in the GC and keep the reference
  /// valid.
  pub fn root(&mut self) {
    if self.root.is_none() {
      self.root = Some(v8::cppgc::Persistent::new(&self.inner));
    }
  }
}

impl<T: GarbageCollected> std::ops::Deref for UnsafePtr<T> {
  type Target = T;
  fn deref(&self) -> &T {
    &unsafe { self.inner.as_ref() }.member
  }
}

#[doc(hidden)]
#[allow(
  clippy::needless_lifetimes,
  reason = "explicit lifetimes improve clarity"
)]
fn try_unwrap_cppgc_with<'sc, T: GarbageCollected + 'static>(
  isolate: &mut v8::Isolate,
  val: v8::Local<'sc, v8::Value>,
  inheriting: &[TypeId],
) -> Option<UnsafePtr<T>> {
  let Ok(obj): Result<v8::Local<v8::Object>, _> = val.try_into() else {
    return None;
  };
  if !obj.is_api_wrapper() {
    return None;
  }

  let obj = unsafe {
    v8::Object::unwrap::<CPPGC_SINGLE_TAG, CppGcObject<T>>(isolate, obj)
  }?;

  let tag = unsafe { obj.as_ref() }.tag;
  if tag != TypeId::of::<T>() && !inheriting.contains(&tag) {
    return None;
  }

  Some(UnsafePtr {
    inner: obj,
    root: None,
  })
}

#[doc(hidden)]
#[allow(
  clippy::needless_lifetimes,
  reason = "explicit lifetimes improve clarity"
)]
pub fn try_unwrap_cppgc_object<'sc, T: GarbageCollected + 'static>(
  isolate: &mut v8::Isolate,
  val: v8::Local<'sc, v8::Value>,
) -> Option<UnsafePtr<T>> {
  try_unwrap_cppgc_with::<T>(isolate, val, &[])
}

pub struct Ref<T: GarbageCollected> {
  inner: v8::cppgc::Persistent<CppGcObject<T>>,
}

impl<T: GarbageCollected> std::ops::Deref for Ref<T> {
  type Target = T;
  fn deref(&self) -> &T {
    &self.inner.get().unwrap().member
  }
}

#[doc(hidden)]
#[allow(
  clippy::needless_lifetimes,
  reason = "explicit lifetimes improve clarity"
)]
pub fn try_unwrap_cppgc_persistent_object<
  'sc,
  T: GarbageCollected + 'static,
>(
  isolate: &mut v8::Isolate,
  val: v8::Local<'sc, v8::Value>,
) -> Option<Ref<T>> {
  let ptr = try_unwrap_cppgc_object::<T>(isolate, val)?;
  Some(Ref {
    inner: v8::cppgc::Persistent::new(&ptr.inner),
  })
}

pub struct Member<T: GarbageCollected> {
  inner: v8::cppgc::Member<CppGcObject<T>>,
}

impl<T: GarbageCollected> From<Ref<T>> for Member<T> {
  fn from(value: Ref<T>) -> Self {
    Member {
      inner: v8::cppgc::Member::new(&value.inner),
    }
  }
}

impl<T: GarbageCollected> std::ops::Deref for Member<T> {
  type Target = T;
  fn deref(&self) -> &T {
    &unsafe { self.inner.get().unwrap() }.member
  }
}

impl<T: GarbageCollected> v8::cppgc::Traced for Member<T> {
  fn trace(&self, visitor: &mut v8::cppgc::Visitor) {
    visitor.trace(&self.inner);
  }
}

#[derive(Debug)]
pub struct SameObject<T: GarbageCollected + 'static> {
  cell: std::cell::OnceCell<v8::Global<v8::Object>>,
  _phantom_data: std::marker::PhantomData<T>,
}

impl<T: GarbageCollected + 'static> SameObject<T> {
  #[allow(
    clippy::new_without_default,
    reason = "Default would hide the intentional construction"
  )]
  pub fn new() -> Self {
    Self {
      cell: Default::default(),
      _phantom_data: Default::default(),
    }
  }
  pub fn get<F>(&self, scope: &mut v8::PinScope, f: F) -> v8::Global<v8::Object>
  where
    F: FnOnce(&mut v8::PinScope) -> T,
  {
    self
      .cell
      .get_or_init(|| {
        let v = f(scope);
        let obj = make_cppgc_object(scope, v);
        v8::Global::new(scope, obj)
      })
      .clone()
  }

  pub fn set(
    &self,
    scope: &mut v8::PinScope,
    value: T,
  ) -> Result<(), v8::Global<v8::Object>> {
    let obj = make_cppgc_object(scope, value);
    self.cell.set(v8::Global::new(scope, obj))
  }

  pub fn try_unwrap(&self, scope: &mut v8::PinScope) -> Option<UnsafePtr<T>> {
    let obj = self.cell.get()?;
    let val = v8::Local::new(scope, obj);
    try_unwrap_cppgc_object(scope, val.cast())
  }
}
