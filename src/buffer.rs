use std::mem::{align_of, size_of};
use std::ops::{Add, Deref};
use std::process::abort;
use std::{mem, ptr};
use std::borrow::Borrow;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::{buffer_mut, BufferCfg, GenericBuffer, ReadableBuffer};
use crate::buffer_mut::BufferMutGeneric;
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, alloc_uninit_buffer, dealloc};

pub type Buffer = BufferGeneric;

#[repr(C)]
pub struct BufferGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = { GROWTH_FACTOR * INLINE_SIZE }, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true, const FAST_CONVERSION: bool = true> {
    len: usize, // the last bit indicates whether the allocation is in-line
    rdx: usize, // the last bit indicates whether the allocation is static
    cap: usize,
    ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
/// the MSB will never be used as allocations are capped at isize::MAX
const STATIC_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
const INLINE_SIZE: usize = size_of::<BufferGeneric::<0, 0, false, false, false>>() - size_of::<usize>() * 2;
const ADDITIONAL_SIZE: usize = size_of::<AtomicUsize>() * 2 - 1;

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Send for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Sync for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}

// FIXME: support static storage!

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Deref for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Borrow<[u8]> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Into<Vec<u8>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn into(mut self) -> Vec<u8> {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            let ptr = &self as *const BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>;
            Vec::from(unsafe { &*slice_from_raw_parts(unsafe { ptr.cast::<u8>().add(size_of::<usize>() * 2) }, self.len()) })
        } else {
            // FIXME: this is wrong!
            unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<Vec<u8>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    fn from(mut value: Vec<u8>) -> Self {
        let mut ptr = value.as_mut_ptr();
        let mut cap = value.capacity();
        let len = value.len();
        let available = cap - len;
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            let mut ret = Self {
                len,
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            };
            let ret_ptr = &mut ret as *mut BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>;
            unsafe { ptr::copy_nonoverlapping(ptr, ret_ptr.cast::<u8>().add(size_of::<usize>() * 2), len); }
            return ret;
        }
        // try reusing existing buffer
        let ref_cnt_ptr = if available < ADDITIONAL_SIZE {
            cap = len + ADDITIONAL_SIZE;
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            unsafe { ptr::copy_nonoverlapping(ptr, alloc, len); }
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(alloc, len) };
            ptr = alloc;
            aligned
        } else {
            mem::forget(value);
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(ptr, len) };
            aligned
        };
        // set reference count to 1
        unsafe { *ref_cnt_ptr.cast::<usize>() = 1; }
        Self {
            len,
            rdx: 0,
            cap,
            ptr,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
GenericBuffer for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_BUFFER_FLAG
            } else {
                0
            },
            rdx: 0,
            cap: 0,
            ptr: null_mut(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        if INLINE_SMALL {
            self.len & !INLINE_BUFFER_FLAG
        } else {
            self.len
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn clear(&mut self) {
        if INLINE_SMALL {
            self.len = 0 | (self.len & INLINE_BUFFER_FLAG);
        } else {
            self.len = 0;
        }
        self.rdx = 0;
    }

    fn shrink(&mut self) {
        todo!()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

    #[inline]
    fn ensure_readable(&self, bytes: usize) -> *const u8 {
        let remaining = self.remaining();
        if remaining < bytes {
            panic!("not enough bytes in buffer, expected {} readable bytes but only {} bytes are left", bytes, remaining);
        }
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        }
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
ReadableBuffer for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<BufferMutGeneric> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    fn from(value: BufferMutGeneric) -> Self {
        debug_assert_eq!(INLINE_SIZE, buffer_mut::INLINE_SIZE);
        if INLINE_SMALL && value.len & INLINE_BUFFER_FLAG != 0 {
            return Self {
                len: value.len,
                rdx: 0,
                cap: value.cap,
                ptr: value.ptr,
            };
        }
        let aligned_len = align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(value.ptr, value.len) + size_of::<AtomicUsize>();
        debug_assert_eq!((value.ptr as usize + aligned_len) % align_of::<AtomicUsize>(), 0);
        if value.cap >= aligned_len {
            // set ref cnt to 1
            unsafe { *value.ptr.add(aligned_len - size_of::<AtomicUsize>()).cast::<usize>() = 1; }
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
        let alloc = unsafe { alloc_uninit_buffer(aligned_len) };
        unsafe { ptr::copy_nonoverlapping(value.ptr, alloc, value.len); }
        let aligned_ptr = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(alloc, value.len) };
        // set ref cnt to 1
        unsafe { *aligned_ptr.cast::<usize>() = 1; }
        Self {
            len: value.len,
            rdx: 0,
            cap: aligned_len,
            ptr: alloc,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
AsRef<[u8]> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Clone for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn clone(&self) -> Self {
        if !INLINE_SMALL || self.len & INLINE_BUFFER_FLAG == 0 {
            let aligned_ptr = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(self.ptr, self.len) };
            // increase the ref cnt if the buffer isn't inlined
            increment_ref_cnt(unsafe { &*aligned_ptr.cast::<AtomicUsize>() });
        }
        Self {
            len: self.len,
            rdx: self.rdx,
            cap: self.cap,
            ptr: self.ptr,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Drop for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    fn drop(&mut self) {
        if !INLINE_SMALL || self.len & INLINE_BUFFER_FLAG == 0 {
            let aligned_ptr = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(self.ptr, self.len) };
            let ref_cnt = unsafe { &*aligned_ptr.cast::<AtomicUsize>() };
            let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
            if remaining == 0 {
                unsafe { dealloc(self.ptr.cast::<u8>(), self.cap); }
            }
        }
    }
}
