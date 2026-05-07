pub(crate) mod sealed {
    pub trait CreateBuffer {}
    impl CreateBuffer for u8 {}
    impl CreateBuffer for i8 {}
    impl CreateBuffer for u16 {}
    impl CreateBuffer for i16 {}
    impl CreateBuffer for u32 {}
    impl CreateBuffer for i32 {}
    impl CreateBuffer for f32 {}
    impl CreateBuffer for f64 {}
    impl CreateBuffer for u64 {}
    impl CreateBuffer for i64 {}
}

/// Helper method to create a V8 BackingStore from a boxed u8 slice,
pub fn v8_create_backing_store<'s, 'i, T: sealed::CreateBuffer>(
  scope: &mut v8::PinScope<'s, 'i, ()>,
  buf: &[T],
  buf_len: usize,
) -> v8::UniqueRef<v8::BackingStore> {
    let backing_store = v8::ArrayBuffer::new_backing_store(scope, buf_len * std::mem::size_of::<T>());
    // Copy the data into the backing store
    unsafe {
        if let Some(data) = backing_store.data() {
        std::ptr::copy(
            buf.as_ptr(),
            data.as_ptr() as *mut T,
            buf_len,
        );
        };
    }
    backing_store
}

/// Helper method to create a array buffer from a boxed u8 slice
pub fn v8_new_array_buffer<'s, 'i, T: sealed::CreateBuffer>(
  scope: &mut v8::PinScope<'s, 'i, ()>,
  buf: &[T],
  buf_len: usize,
) -> v8::Local<'s, v8::ArrayBuffer> {
    if buf.is_empty() {
        return v8::ArrayBuffer::new(scope, 0);
    }
    let bs = v8_create_backing_store(scope, buf, buf_len);
    v8::ArrayBuffer::with_backing_store(scope, &bs.make_shared())
}