use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::{Deref, RangeBounds};
use std::{mem, ptr};
use std::cmp::Ord;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, alloc_uninit_buffer, alloc_zeroed_buffer, build_bit_mask, dealloc, empty_sentinel, find_sufficient_cap, min, realloc_buffer, realloc_buffer_and_dealloc, realloc_buffer_counted};
use crate::{buffer, buffer_mut, GenericBuffer, ReadableBuffer, RWBuffer, WritableBuffer};
use crate::buffer::BufferGeneric;
use crate::buffer_mut::BufferMutGeneric;

pub type BufferRW = BufferRWGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

const LEN_MASK: usize = build_bit_mask(0, 5);
const RDX_MASK: usize = build_bit_mask(5, 5);

#[repr(C)]
pub struct BufferRWGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true> {
    pub(crate) len: usize,
    pub(crate) rdx: usize,
    pub(crate) cap: usize,
    pub(crate) ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
/// the MSB will never be used as allocations are capped at isize::MAX
const STATIC_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferRWGeneric<0, 0, false, false>>() - size_of::<usize>();
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer_mut::BASE_INLINE_SIZE), buffer::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// to align said values properly.
const ADDITIONAL_BUFFER_CAP: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Send for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Sync for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

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
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        let self_ptr = self as *mut BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }>;
        if self.is_inlined() {
            if self.len() + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>(buffer: *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE>, req: usize) -> *mut u8 {
                    let rdx = (unsafe { (&*buffer).len } & RDX_MASK) >> LEN_MASK.count_ones();
                    // remove the inline flag and rdx data
                    unsafe { (&mut *buffer).len &= !(INLINE_BUFFER_FLAG | RDX_MASK); }
                    let len = unsafe { (&*buffer).len };
                    let cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(INITIAL_CAP, len + req + ADDITIONAL_BUFFER_CAP);
                    let alloc = unsafe { realloc_buffer_counted(buffer.cast::<u8>().add(size_of::<usize>()), len, cap) };

                    unsafe { (&mut *buffer).cap = cap };
                    unsafe { (&mut *buffer).ptr = alloc };
                    unsafe { (&mut *buffer).rdx = rdx };
                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(self_ptr, req);
            }
            return unsafe { self_ptr.cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // move the static buffer into a dynamic heap buffer
        if self.is_static() {
            let cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(INITIAL_CAP, self.len + req + ADDITIONAL_BUFFER_CAP);
            let alloc = unsafe { realloc_buffer_counted(self.ptr, self.len, cap) };
            self.ptr = alloc;
            self.cap = cap;
            // mark the buffer as non static
            self.rdx &= !STATIC_BUFFER_FLAG;
            return unsafe { self.ptr.add(self.len) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req + ADDITIONAL_BUFFER_CAP {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>(buffer: *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE>, req: usize) {
                let old_cap = unsafe { (&*buffer).cap };
                let len = unsafe { (&*buffer).len };
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(old_cap, len + req + ADDITIONAL_BUFFER_CAP);
                let new_alloc = unsafe { realloc_buffer_and_dealloc((&*buffer).ptr, len, old_cap, new_cap) };
                unsafe { (&mut *buffer).ptr = new_alloc; }
                unsafe { (&mut *buffer).cap = new_cap; }
                // set ref cnt
                unsafe { *(&*buffer).meta_ptr().cast::<usize>() = 1; }
            }
            resize_alloc(self_ptr, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

    #[inline]
    fn ensure_readable(&self, bytes: usize) -> *const u8 {
        let remaining = self.remaining();
        if remaining < bytes {
            panic!("not enough bytes in buffer, expected {} readable bytes but only {} bytes are left", bytes, remaining);
        }
        let rdx = if STATIC_STORAGE {
            self.rdx & !STATIC_BUFFER_FLAG
        } else {
            self.rdx
        };
        if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr().add(rdx) }
        } else {
            unsafe { self.ptr.add(rdx) }
        }
    }

    /// SAFETY: this may only be called if the buffer isn't
    /// inlined and isn't a static buffer
    #[inline]
    unsafe fn meta_ptr(&self) -> *mut u8 {
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(self.ptr, self.cap) }
    }

    /// SAFETY: this may only be called if the buffer is inlined.
    #[inline]
    unsafe fn inlined_buffer_ptr(&self) -> *mut u8 {
        let ptr = self as *const BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }>;
        unsafe { ptr.cast::<u8>().add(size_of::<usize>()) }.cast_mut()
    }

    #[inline]
    fn get_rdx(&self) -> usize {
        if self.is_inlined() {
            return (self.len & RDX_MASK) >> LEN_MASK.count_ones();
        }
        if STATIC_STORAGE {
            self.rdx & !STATIC_BUFFER_FLAG
        } else {
            self.rdx
        }
    }

    #[inline]
    fn set_rdx(&mut self, rdx: usize) {
        if self.is_inlined() {
            self.len = (self.len & !RDX_MASK) | (rdx << LEN_MASK.count_ones());
            return;
        }
        let flags = if STATIC_STORAGE {
            self.rdx & STATIC_BUFFER_FLAG
        } else {
            0
        };
        self.rdx = rdx | flags;
    }

    #[inline]
    fn set_len(&mut self, len: usize) {
        let flags = if INLINE_SMALL {
            self.len & (INLINE_BUFFER_FLAG | RDX_MASK)
        } else {
            0
        };
        self.len = len | flags;
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
GenericBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn new() -> Self {
        if !INLINE_SMALL && STATIC_STORAGE {
            static EMPTY: &[u8] = &[];
            return Self::from(EMPTY);
        }

        Self {
            len: if INLINE_SMALL {
                0 | INLINE_BUFFER_FLAG
            } else {
                0
            },
            rdx: 0,
            ptr: if INLINE_SMALL {
                null_mut()
            } else {
                empty_sentinel()
            },
            cap: 0,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        if self.is_inlined() {
            return self.len & LEN_MASK;
        }
        self.len
    }

    #[inline]
    fn clear(&mut self) {
        self.len &= !INLINE_BUFFER_FLAG;
        self.rdx = 0;
    }

    fn shrink(&mut self) {
        if self.is_inlined() {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        if self.is_static() {
            // we have nothing to do as the buffer is static
            return;
        }
        if !unsafe { self.is_only() } {
            // for now we just nop if there are other references to the buffer
            return;
        }
        let target_cap = self.len + ADDITIONAL_BUFFER_CAP;
        if self.cap >= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        let alloc = unsafe { realloc_buffer_counted(self.ptr, self.len, target_cap) };
        let old_buf = self.ptr;
        unsafe { dealloc(old_buf, self.cap); }
        self.ptr = alloc;
        self.cap = target_cap;
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        if self.len() > len {
            self.set_len(len);
            // fixup rdx after updating len in order to avoid the rdx getting OOB
            self.set_rdx(self.get_rdx().min(len));
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
WritableBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_BUFFER_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                ptr: null_mut(),
                cap: 0,
            }
        } else {
            let cap = cap + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            let ret = Self {
                len: 0,
                rdx: 0,
                ptr: alloc,
                cap,
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
    }

    fn zeroed(len: usize) -> Self {
        if INLINE_SMALL && len <= INLINE_SIZE {
            Self {
                len: len | INLINE_BUFFER_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                ptr: null_mut(),
                cap: 0,
            }
        } else {
            let cap =  len + ADDITIONAL_BUFFER_CAP;
            let alloc = alloc_zeroed_buffer(cap);
            let ret = Self {
                len,
                rdx: 0,
                ptr: alloc,
                cap,
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
    }

    fn reserve(&mut self, size: usize) {
        if self.is_inlined() {
            if size <= INLINE_SIZE {
                return;
            }
            let new_size = {
                let mut new_size = INITIAL_CAP * GROWTH_FACTOR;
                // if cap * GROWTH_FACTOR is smaller than size, retry
                while new_size < size {
                    new_size *= GROWTH_FACTOR;
                }
                new_size
            };
            let alloc = unsafe { alloc_uninit_buffer(new_size) };
            let inlined_ptr = unsafe { (self as *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> as *mut u8).add(size_of::<usize>()) };
            unsafe { ptr::copy_nonoverlapping(inlined_ptr, alloc, self.len()); }
            return;
        }
        let cap = self.cap;
        let len = self.len();
        let remaining = cap - len;
        if remaining < size {
            let new_size = {
                let mut new_size = cap * GROWTH_FACTOR;
                // if cap * GROWTH_FACTOR is smaller than size, retry
                while new_size < size {
                    new_size *= GROWTH_FACTOR;
                }
                new_size
            };
            let alloc = unsafe { alloc_uninit_buffer(new_size) };
            unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, len); }
            unsafe { dealloc(self.ptr, cap); }
            self.ptr = alloc;
            self.cap = size;
        }
    }

    fn resize(&mut self, size: usize) {
        
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if self.is_inlined() {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn put_bytes(&mut self, val: &[u8]) {
        let ptr = self.ensure_large_enough(val.len());
        unsafe { ptr::copy_nonoverlapping(val as *const [u8] as *const u8, ptr, val.len()); }
        self.len += val.len();
    }

    #[inline]
    fn put_u8(&mut self, val: u8) {
        let ptr = self.ensure_large_enough(1);
        unsafe { *ptr = val; }
        self.len += 1;
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
ReadableBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn remaining(&self) -> usize {
        let rdx = if STATIC_STORAGE {
            self.rdx & !STATIC_BUFFER_FLAG
        } else {
            self.rdx
        };
        self.len() - rdx
    }

    #[inline]
    fn split_off(&mut self, offset: usize) -> Self {
        let idx = self.get_rdx() + offset;
        assert!(self.len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        self.set_len(idx);
        other.set_rdx(idx);

        other
    }

    fn split_to(&mut self, offset: usize) -> Self {
        let idx = self.get_rdx() + offset;
        assert!(self.len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        other.set_len(idx);
        self.set_rdx(idx);

        other
    }

    fn split(&mut self) -> Self {
        // TODO: check if the panic check gets removed
        self.split_off(0)
    }

    fn unsplit(&mut self, other: Self) {
        if self.is_empty() {
            *self = other;
            return;
        }

        if self.is_static() {
            if !other.is_static() {
                panic!("Static buffers can only be merged with other static buffers");
            }
            if self.ptr != other.ptr {
                panic!("Unsplitting only works on buffers that have the same src");
            }
            if self.get_rdx() + self.len() != other.get_rdx() && self.len() != other.get_rdx() + other.len() {
                panic!("Unsplitting only works on buffers that are next to each other");
            }
            self.set_rdx(self.get_rdx().min(other.get_rdx()));
            self.set_len(self.len() + other.len());
            return;
        }

        if self.is_inlined() {
            if !other.is_inlined() {
                panic!("Inlined buffers can only be merged with other inlined buffers");
            }
            if self.remaining() + other.remaining() > INLINE_SIZE {
                panic!("Unsplitting inlined buffers only works if they are small enough");
            }
            if self.get_rdx() + self.len() != other.get_rdx() && self.get_rdx() != other.get_rdx() + other.len() {
                panic!("Unsplitting only works on buffers that are next to each other");
            }
            self.set_rdx(self.get_rdx().min(other.get_rdx()));
            self.set_len(self.len().max(other.len()));
            return;
        }

        if self.ptr != other.ptr {
            panic!("Unsplitting only works on buffers that have the same src");
        }
        if self.get_rdx() + self.len() != other.get_rdx() && self.get_rdx() != other.get_rdx() + other.len() {
            panic!("Unsplitting only works on buffers that are next to each other");
        }
        self.set_rdx(self.get_rdx().min(other.get_rdx()));
        self.set_len(self.len().max(other.len()));
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
RWBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Drop for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if self.is_static() {
            // we don't need to do anything for static buffers
            return;
        }
        if !INLINE_SMALL && !STATIC_STORAGE && self.ptr == empty_sentinel() {
            // we don't do anything for empty buffers
            return;
        }
        // fast path for single ref cnt scenarios
        if unsafe { self.is_only() } {
            unsafe { dealloc(self.ptr, self.cap); }
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // TODO: can we choose a weaker ordering?
        if remaining == 0 {
            unsafe { dealloc(self.ptr, self.cap); }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Clone for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_inlined() || self.is_static() {
            return Self {
                len: self.len,
                rdx: self.rdx,
                ptr: self.ptr,
                cap: self.cap,
            };
        }

        // we can't just increment reference count as that would allow for
        // multiple mutable references to the same memory location

        let alloc = unsafe { realloc_buffer_counted(self.ptr, self.len, self.cap) };

        Self {
            len: self.len,
            rdx: self.rdx,
            ptr: alloc,
            cap: self.cap,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
AsRef<[u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr() }
        } else {
            self.ptr
        };
        let ptr = unsafe { ptr.add(self.rdx) };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Deref for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Borrow<[u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Default for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<&'static [u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self {
            len: value.len(),
            rdx: 0 | STATIC_BUFFER_FLAG,
            ptr: value.as_ptr().cast_mut(),
            cap: value.len(),
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<Vec<u8>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> Vec<u8> {
        if self.is_inlined() {
            let alloc = unsafe { realloc_buffer(self.inlined_buffer_ptr(), self.len(), self.len()) }; // FIXME: should we add ADDITIONAL_BUFFER_CAP?
            return unsafe { Vec::from_raw_parts(alloc, self.len(), self.len()) };
        }
        if self.is_static() {
            let cap = self.len + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { realloc_buffer(self.ptr, self.len, cap) };
            return unsafe { Vec::from_raw_parts(alloc, self.len, cap) };
        }
        if unsafe { self.is_only() } {
            let ret = unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) };
            mem::forget(self);
            return ret;
        }
        // TODO: should we try to shrink?
        let buf = unsafe { realloc_buffer(self.ptr, self.len, self.cap) };
        unsafe { Vec::from_raw_parts(buf, self.len, self.cap) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<Vec<u8>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            // FIXME: should we instead keep the small buffer if it exists already and doesn't cost us anything?
            let mut ret = Self {
                len: len | INLINE_BUFFER_FLAG,
                rdx: 0,
                ptr: null_mut(),
                cap: 0,
            };
            unsafe { ptr::copy_nonoverlapping(ptr, ret.inlined_buffer_ptr(), len); }
            return ret;
        }
        mem::forget(value);
        // reuse existing buffer
        let ret = Self {
            len,
            rdx: 0,
            ptr,
            cap,
        };
        // set ref cnt
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        ret
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(value: BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE>) -> Self {
        if value.is_inlined() || value.is_static() {
            return Self {
                len: value.len,
                rdx: value.rdx,
                ptr: value.ptr,
                cap: 0,
            };
        }
        if unsafe { value.is_only() } {
            let ret = Self {
                len: value.len,
                rdx: value.rdx,
                ptr: value.ptr,
                cap: value.capacity(),
            };
            mem::forget(value);
            return ret;
        }
        // TODO: should we try to shrink?
        let cap = value.capacity();
        let alloc = unsafe { realloc_buffer_counted(value.ptr, value.len, cap) };
        Self {
            len: value.len,
            rdx: value.rdx,
            ptr: alloc,
            cap,
        }
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL> {
        if self.is_inlined() || self.is_static() {
            return BufferGeneric {
                len: self.len,
                rdx: self.rdx,
                ptr: self.ptr,
                cap: self.cap,
            };
        }
        if unsafe { self.is_only() } {
            let ret = BufferGeneric {
                len: self.len,
                rdx: self.rdx,
                cap: self.cap,
                ptr: self.ptr,
            };
            mem::forget(self);
            return ret;
        }

        // TODO: should we try to shrink?
        let alloc = unsafe { realloc_buffer_counted(self.ptr, self.len, self.cap) };
        BufferGeneric {
            len: self.len,
            rdx: self.rdx,
            cap: self.cap,
            ptr: alloc,
        }
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(value: BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>) -> Self {
        let ret = Self {
            len: value.len,
            rdx: 0,
            ptr: value.ptr,
            cap: value.cap,
        };
        mem::forget(value);
        ret
    }
}

impl
Into<BufferMutGeneric> for BufferRWGeneric {
    #[inline]
    fn into(self) -> BufferMutGeneric {
        let ret = BufferMutGeneric {
            len: self.len,
            ptr: self.ptr,
            cap: self.cap,
            offset: self.cap,
        };
        mem::forget(self);
        ret
    }
}
