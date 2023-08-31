use std::mem::{align_of, size_of};
use std::ops::{Deref, RangeBounds};
use std::process::abort;
use std::{mem, ptr};
use std::borrow::Borrow;
use std::cmp::Ord;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::{buffer_mut, buffer_rw, GenericBuffer, ReadableBuffer};
use crate::buffer_mut::BufferMutGeneric;
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, dealloc, min, realloc_buffer};

pub type Buffer = BufferGeneric;

#[repr(C)]
pub struct BufferGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = { GROWTH_FACTOR * INLINE_SIZE }, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true> {
    pub(crate) len: usize, // the last bit indicates whether the allocation is in-line
    pub(crate) rdx: usize, // the last bit indicates whether the allocation is static
    pub(crate) alloc_len: usize,
    pub(crate) ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
/// the MSB will never be used as allocations are capped at isize::MAX
const STATIC_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferGeneric<0, 0, false, false>>() - size_of::<usize>() * 2;
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer_mut::BASE_INLINE_SIZE), buffer_rw::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// capacity and to align said values properly.
const ADDITIONAL_SIZE: usize = size_of::<AtomicUsize>() * 3 - 1;

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Send for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Sync for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Deref for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Borrow<[u8]> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<Vec<u8>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> Vec<u8> {
        // handle inlined buffers
        if self.is_inlined() {
            return Vec::from(unsafe { &*slice_from_raw_parts(self.inlined_buffer_ptr(), self.len()) });
        }

        if self.is_static() {
            // try reusing buffer
            if unsafe { self.is_only() } {
                let ret = unsafe { Vec::from_raw_parts(self.ptr, self.len, self.capacity()) };
                mem::forget(self);
                return ret;
            }
        }
        // TODO: should we try to shrink?
        let cap = self.capacity();
        let alloc = unsafe { realloc_buffer(self.ptr, self.len, cap) };
        unsafe { Vec::from_raw_parts(alloc, cap, self.len) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<Vec<u8>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn from(mut value: Vec<u8>) -> Self {
        let mut ptr = value.as_mut_ptr();
        let mut cap = value.capacity();
        let len = value.len();
        let available = cap - len;
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            let mut ret = Self {
                len: len | INLINE_BUFFER_FLAG,
                rdx: 0,
                alloc_len: 0,
                ptr: null_mut(),
            };
            unsafe { ptr::copy_nonoverlapping(ptr, ret.inlined_buffer_ptr(), len); }
            return ret;
        }
        // try reusing existing buffer
        let ref_cnt_ptr = if available < ADDITIONAL_SIZE {
            cap = len + ADDITIONAL_SIZE;
            let alloc = unsafe { realloc_buffer(ptr, len, cap) };
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(alloc, len) };
            ptr = alloc;
            aligned
        } else {
            mem::forget(value);
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(ptr, len) };
            aligned
        };
        // init metadata
        unsafe { *ref_cnt_ptr.cast::<usize>() = 1; }
        unsafe { *ref_cnt_ptr.cast::<usize>().offset(1) = cap; }
        Self {
            len,
            rdx: 0,
            alloc_len: len,
            ptr,
        }
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn from(value: BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>) -> Self {
        if value.is_inlined() {
            return Self {
                len: value.len,
                rdx: 0,
                alloc_len: value.alloc_len,
                ptr: value.ptr,
            };
        }
        let aligned_len = align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(value.ptr, value.len) + size_of::<AtomicUsize>();
        debug_assert_eq!((value.ptr as usize + aligned_len) % align_of::<AtomicUsize>(), 0);
        // reuse the buffer if this instance is the only one.
        if unsafe { value.is_only() } {
            let ret = Self {
                len: value.len,
                rdx: 0,
                alloc_len: value.alloc_len,
                ptr: value.ptr,
            };
            // set the capacity metadata
            unsafe { *ret.meta_ptr().cast::<usize>().add(1) = value.cap; }
            mem::forget(value);
            return ret;
        }
        #[inline(never)]
        #[cold]
        fn resize_alloc(ptr: *mut u8, len: usize) -> *mut u8 {
            let cap = len + ADDITIONAL_SIZE;
            let alloc = unsafe { realloc_buffer(ptr, len, cap) };
            let aligned_ptr = unsafe { align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(alloc, len) };
            // init metadata
            unsafe { *aligned_ptr.cast::<usize>() = 1; }
            unsafe { *aligned_ptr.cast::<usize>().add(1) = cap; }
            alloc
        }
        let alloc = resize_alloc(value.ptr, value.len);
        let ret = Self {
            len: value.len,
            rdx: 0,
            alloc_len: value.alloc_len,
            ptr: alloc,
        };
        // initialize metadata
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        unsafe { *ret.meta_ptr().cast::<usize>().add(1) = value.cap; }
        ret
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL> {
        if self.is_inlined() {
            return BufferMutGeneric {
                len: self.len,
                alloc_len: self.alloc_len,
                ptr: self.ptr,
                cap: self.capacity(),
            };
        }
        if self.is_static() {
            let cap = self.len + ADDITIONAL_SIZE;
            let alloc = unsafe { realloc_buffer(self.ptr, self.len, cap) };
            let ret = BufferMutGeneric {
                len: self.len,
                alloc_len: self.alloc_len,
                ptr: alloc,
                cap,
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            return ret;
        }
        if unsafe { self.is_only() } {
            let ret = BufferMutGeneric {
                len: self.len,
                alloc_len: self.alloc_len,
                ptr: self.ptr,
                cap: self.capacity(),
            };
            mem::forget(self);
            return ret;
        }
        let alloc = unsafe { realloc_buffer(self.ptr, self.len, self.len + ADDITIONAL_SIZE) };
        
        let ret = BufferMutGeneric {
            len: self.len,
            alloc_len: self.alloc_len,
            ptr: alloc,
            cap: self.len + ADDITIONAL_SIZE,
        };
        // set ref cnt
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        ret
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<&'static [u8]> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self {
            len: value.len(),
            rdx: 0 | STATIC_BUFFER_FLAG,
            alloc_len: value.len(),
            ptr: value.as_ptr().cast_mut(),
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
GenericBuffer for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_BUFFER_FLAG
            } else {
                0
            },
            rdx: 0,
            alloc_len: 0,
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
    fn clear(&mut self) {
        if INLINE_SMALL {
            self.len = 0 | (self.len & INLINE_BUFFER_FLAG);
        } else {
            self.len = 0;
        }
        self.rdx = 0;
    }

    /// this can lead to a second buffer being allocated while the first buffer staying
    /// alive. this can happen if the reference count is larger than 1.
    fn shrink(&mut self) {
        if self.is_inlined() {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        if self.is_static() {
            // we have nothing to do for static buffers
            return;
        }
        if !unsafe { self.is_only() } {
            // For now we just nop for buffers we don't completely own.
            return;
        }
        let target_cap = self.len + ADDITIONAL_SIZE;
        if self.capacity() <= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        let alloc = unsafe { realloc_buffer(self.ptr, self.len, target_cap) };
        let old = self.ptr;
        let cap = self.capacity();
        unsafe { dealloc(old, cap); }
        self.ptr = alloc;
        // set metadata
        unsafe { *self.meta_ptr().cast::<usize>() = 1; }
        unsafe { *self.meta_ptr().cast::<usize>().add(1) = target_cap; }
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        if self.len() > len {
            if INLINE_SMALL {
                self.len = len | (self.len & INLINE_BUFFER_FLAG);
            } else {
                self.len = len;
            }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

    #[inline]
    pub(crate) fn is_static(&self) -> bool {
        STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0
    }

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0
    }

    /// SAFETY: this is only safe to call if the buffer isn't inlined and isn't static.
    #[inline]
    pub(crate) unsafe fn is_only(&self) -> bool {
        let meta_ptr = unsafe { self.meta_ptr() };
        unsafe { &*meta_ptr.cast::<AtomicUsize>() }.load(Ordering::Acquire) == 1
    }

    #[inline]
    fn ensure_readable(&self, bytes: usize) -> *const u8 {
        let remaining = self.remaining();
        if remaining < bytes {
            panic!("not enough bytes in buffer, expected {} readable bytes but only {} bytes are left", bytes, remaining);
        }
        let rdx = self.raw_rdx();
        if self.is_inlined() {
            let ptr = self as *const BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE>;
            unsafe { ptr.cast::<u8>().add(size_of::<usize>() * 2 + rdx) }
        } else {
            unsafe { self.ptr.add(rdx) }
        }
    }

    #[inline]
    fn raw_rdx(&self) -> usize {
        if STATIC_STORAGE {
            self.rdx & !STATIC_BUFFER_FLAG
        } else {
            self.rdx
        }
    }

    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if self.is_inlined() {
            return INLINE_SIZE;
        }
        if self.is_static() {
            return self.alloc_len;
        }
        let ptr = unsafe { self.meta_ptr().cast::<usize>().add(1) };
        unsafe { *ptr }
    }

    /// SAFETY: this may only be called if the buffer isn't
    /// inlined and isn't a static buffer
    #[inline]
    pub(crate) unsafe fn meta_ptr(&self) -> *mut u8 {
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }>(self.ptr, self.alloc_len) }
    }

    /// SAFETY: this may only be called if the buffer is inlined.
    #[inline]
    unsafe fn inlined_buffer_ptr(&self) -> *mut u8 {
        let ptr = self as *const BufferGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }>;
        unsafe { ptr.cast::<u8>().add(size_of::<usize>() * 2) }.cast_mut()
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
ReadableBuffer for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

    #[inline]
    fn remaining(&self) -> usize {
        self.len() - self.raw_rdx()
    }

    #[inline]
    fn split_off(&mut self, offset: usize) -> Self {
        let idx = self.raw_rdx() + offset;
        assert!(self.len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        let len_flag = if INLINE_SMALL {
            self.len & INLINE_BUFFER_FLAG
        } else {
            0
        };
        let rdx_flag = if STATIC_STORAGE {
            self.rdx & STATIC_BUFFER_FLAG
        } else {
            0
        };

        self.len = idx | len_flag;
        other.rdx = idx | rdx_flag;

        other
    }

    #[inline]
    fn split_to(&mut self, offset: usize) -> Self {
        let idx = self.raw_rdx() + offset;
        assert!(self.len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        let rdx_flag = if STATIC_STORAGE {
            self.rdx & STATIC_BUFFER_FLAG
        } else {
            0
        };
        let len_flag = if INLINE_SMALL {
            self.len & INLINE_BUFFER_FLAG
        } else {
            0
        };

        self.rdx = idx | rdx_flag;
        other.len = idx | len_flag;

        other
    }

    #[inline]
    fn split(&mut self) -> Self {
        // TODO: check if the panic check gets removed
        self.split_off(0)
    }

    fn unsplit(&mut self, other: Self) {
        if self.is_empty() {
            *self = other;
            return;
        }

        todo!()
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
AsRef<[u8]> for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr() }
        } else {
            self.ptr
        };
        let rdx = self.raw_rdx();
        let ptr = unsafe { ptr.add(rdx) };
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Clone for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn clone(&self) -> Self {
        if !self.is_inlined() {
            let meta_ptr = unsafe { self.meta_ptr() };
            // increase the ref cnt if the buffer isn't inlined
            increment_ref_cnt(unsafe { &*meta_ptr.cast::<AtomicUsize>() });
        }
        Self {
            len: self.len,
            rdx: self.rdx,
            alloc_len: self.alloc_len,
            ptr: self.ptr,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Drop for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if self.is_static() {
            // we don't need to do anything for static buffers
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            let cap = self.capacity();
            unsafe { dealloc(self.ptr.cast::<u8>(), cap); }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Default for BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
