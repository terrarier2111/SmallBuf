use std::alloc::{alloc, dealloc, Layout};
use std::mem::{align_of, size_of};
use std::ops::Add;
use std::process::abort;
use std::{mem, ptr};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::{buffer_mut, GenericBuffer, ReadableBuffer};
use crate::buffer_mut::BufferMut;
use crate::util::{align_to, alloc_uninit_buffer};

#[repr(C)]
pub struct Buffer {
    len: usize,
    rdx: usize,
    cap: usize,
    ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
const INLINE_SIZE: usize = size_of::<Buffer>() - size_of::<usize>() * 2;

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl GenericBuffer for Buffer {
    #[inline]
    fn new() -> Self {
        Self {
            len: 0 | INLINE_BUFFER_FLAG,
            rdx: 0,
            cap: 0,
            ptr: null_mut(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len & !INLINE_BUFFER_FLAG
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if self.len & INLINE_BUFFER_FLAG != 0 {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn clear(&mut self) {
        self.len = 0 | (self.len & INLINE_BUFFER_FLAG);
        self.rdx = 0;
    }

    fn shrink(&mut self) {
        todo!()
    }
}

impl Buffer {

    #[inline]
    fn ensure_readable(&self, bytes: usize) -> *const u8 {
        let remaining = self.remaining();
        if remaining < bytes {
            panic!("not enough bytes in buffer, expected {} readable bytes but only {} bytes are left", bytes, remaining);
        }
        if self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const Buffer).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        }
    }

}

impl ReadableBuffer for Buffer {

    #[inline]
    fn remaining(&self) -> usize {
        self.len() - self.rdx
    }

    #[inline]
    fn get_bytes(&mut self, bytes: usize) -> &[u8] {
        let ptr = self.ensure_readable(bytes);
        self.rdx += bytes;
        unsafe { &*slice_from_raw_parts(ptr, bytes) }
    }

    #[inline]
    fn get_u8(&mut self) -> u8 {
        let ptr = self.ensure_readable(1);
        self.rdx += 1;
        unsafe { *ptr }
    }

}

const MAX_REF_CNT: usize = usize::MAX / 2;

impl From<BufferMut> for Buffer {
    fn from(value: BufferMut) -> Self {
        assert_eq!(INLINE_SIZE, buffer_mut::INLINE_SIZE);
        if value.len() <= INLINE_SIZE {
            return Self {
                len: value.len,
                rdx: 0,
                cap: value.cap,
                ptr: value.ptr,
            };
        }
        let aligned_len = align_to::<{ align_of::<AtomicUsize>() }>(value.len);
        assert_eq!(aligned_len % align_of::<AtomicUsize>(), 0);
        if value.cap >= aligned_len + size_of::<AtomicUsize>() {
            unsafe { *value.ptr.add(aligned_len).cast::<usize>() = 1; }
            let ret = Self {
                len: value.len,
                rdx: 0,
                cap: value.cap,
                ptr: value.ptr,
            };
            mem::forget(value);
            return ret;
        }
        // FIXME: make this cold!
        let alloc = unsafe { alloc_uninit_buffer(aligned_len + size_of::<AtomicUsize>()) };
        unsafe { ptr::copy_nonoverlapping(value.ptr, alloc, value.len); }
        unsafe { *value.ptr.add(aligned_len).cast::<usize>() = 1; }
        Self {
            len: value.len,
            rdx: 0,
            cap: aligned_len + size_of::<AtomicUsize>(),
            ptr: alloc,
        }
    }
}

impl AsRef<[u8]> for Buffer {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const Buffer).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

#[inline]
fn increment_ref_cnt(ref_cnt: &AtomicUsize) {
    let val = ref_cnt.fetch_add(1, Ordering::AcqRel); // FIXME: can we choose a weaker ordering?
    if val > MAX_REF_CNT {
        abort();
    }
}

impl Clone for Buffer {
    #[inline]
    fn clone(&self) -> Self {
        if self.len & INLINE_BUFFER_FLAG == 0 {
            // increase the ref cnt if the buffer isn't inlined
            increment_ref_cnt(unsafe { &*self.ptr.add(self.len).cast::<AtomicUsize>() });
        }
        Self {
            len: self.len,
            rdx: self.rdx,
            cap: self.cap,
            ptr: self.ptr,
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if self.len & INLINE_BUFFER_FLAG == 0 {
            let aligned_len = align_to::<{ align_of::<AtomicUsize>() }>(self.len);
            let ref_cnt = unsafe { &*self.ptr.add(aligned_len).cast::<AtomicUsize>() };
            let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
            if remaining == 0 {
                unsafe { dealloc(self.ptr.cast::<u8>(), Layout::from_size_align_unchecked(self.cap, align_of::<AtomicUsize>())); }
            }
        }
    }
}
