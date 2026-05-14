use std::ops::{Deref, DerefMut};

use v8::SharedRef;

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

pub fn v8_backing_store_to_vec(bs: SharedRef<v8::BackingStore>, offset: usize, len: usize) -> Vec<u8> {
    let dest = match bs.data() {
        Some(ptr) => {
            let p = ptr.as_ptr() as *const u8;
            let mut dest = vec![0u8; len];
            unsafe {
                std::ptr::copy_nonoverlapping(p.add(offset), dest.as_mut_ptr(), len);
            }
            dest
        },
        None => Vec::with_capacity(0)
    };

    dest
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

/// A safe, zero-copy wrapper around V8 ArrayBuffer memory.
pub struct ZeroCopyBuf<'s> {
    /// Ensure the buffer is never garbage copied
    _backing_store: v8::SharedRef<v8::BackingStore>,
    
    /// The actual raw slice tied to the lifetime of this struct
    data: &'s mut [u8],
}

impl<'s> ZeroCopyBuf<'s> {
    /// Attempts to extract a zero-copy slice from a v8::Value
    /// 
    /// SAFETY: 
    /// - This API should only be called from within callbacks with DisallowJavascriptExecutionScope
    /// - Do not attempt to replace the underlying slice whatsoever, only mutate
    pub fn try_from_v8(
        _scope: &mut v8::PinnedRef<'_, v8::DisallowJavascriptExecutionScope<'_, 's, v8::HandleScope<'_>>>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<Self> {
        if value.is_array_buffer_view() {
            let view = v8::Local::<v8::ArrayBufferView>::try_from(value).unwrap();
            let Some(backing_store) = view.get_backing_store() else {
                return None; // detached view
            };
            let byte_length = view.byte_length();

            let data = match backing_store.data() {
                Some(ptr) if byte_length > 0 => {
                    let data_ptr = ptr.as_ptr() as *mut u8; 
                    // SAFETY: v8 guarantees that the pointer is correctly aligned (i think?)
                    unsafe {
                        std::slice::from_raw_parts_mut(
                            data_ptr.add(view.byte_offset()),
                            byte_length,
                        )
                    }
                }
                _ => &mut [], // Length is 0
            };

            return Some(Self {
                _backing_store: backing_store,
                data,
            });
        } else if value.is_array_buffer() {
            let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value).unwrap();
            let backing_store = buffer.get_backing_store();
            let byte_length = buffer.byte_length();
            
            let data = match backing_store.data() {
                Some(ptr) if byte_length > 0 => {
                    let data_ptr = ptr.as_ptr() as *mut u8; 
                    // SAFETY: v8 guarantees that the pointer is correctly aligned (i think?)
                    unsafe {
                        std::slice::from_raw_parts_mut(
                            data_ptr,
                            byte_length,
                        )
                    }
                }
                _ => &mut [], // Length is 0
            };

            return Some(Self {
                _backing_store: backing_store,
                data,
            });
        }

        None
    }

    /// 'Safely' casts the internal byte buffer to a typed mutable slice (e.g., u32, f32).
    /// Returns an error if the buffer is improperly aligned or the wrong size.
    pub fn as_mut_slice_of<T>(&mut self) -> Result<&mut [T], &'static str> {
        let size = std::mem::size_of::<T>();

        if self.data.len() % size != 0 {
            return Err("Buffer length is not perfectly divisible by the target type size");
        }

        let (prefix, aligned_slice, suffix) = unsafe { self.data.align_to_mut::<T>() };

        if !prefix.is_empty() || !suffix.is_empty() {
            return Err("Buffer memory address is misaligned for the requested type");
        }

        Ok(aligned_slice)
    }
}

impl<'s> Deref for ZeroCopyBuf<'s> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'s> DerefMut for ZeroCopyBuf<'s> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}