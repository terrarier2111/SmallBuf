use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::Deref;
use std::{mem, ptr};
use std::cmp::Ord;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::{buffer, buffer_rw, GenericBuffer, WritableBuffer};
use crate::buffer_rw::BufferRWGeneric;
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, alloc_uninit_buffer, alloc_zeroed_buffer, build_bit_mask, dealloc, empty_sentinel, find_sufficient_cap, min, realloc_buffer, realloc_buffer_and_dealloc, realloc_buffer_counted};

pub type BufferMut = BufferMutGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

const LEN_MASK: usize = build_bit_mask(0, 5);
const OFFSET_MASK: usize = build_bit_mask(5, 5);

#[repr(C)]
pub struct BufferMutGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true> {
    pub(crate) len: usize,
    pub(crate) wrx: usize, // this indicates the write index
    pub(crate) offset: usize, // this is an offset into the allocation
    pub(crate) ptr: *mut u8, // this points to the end of the allocation minus allocation size (in order to use this, one has to mask off the lower bits)
}

// FIXME: only allow cap to be a multiple of meta_align in order to be able to use the lower bits to store the additional size that was masked off to align the metadata properly

// FIXME: store base ptr and alloc_cap in the metadata in addition to the ref cnt


// TODO: additional features: allow aligning the ref cnt ptr to the cache line size

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_FLAG: usize = 1 << (usize::BITS - 1);
/// this is only
pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferMutGeneric<0, 0, false>>() - size_of::<usize>();
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer::BASE_INLINE_SIZE), buffer_rw::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// to align said values properly.
const ADDITIONAL_BUFFER_CAP: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Send for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Sync for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
GenericBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_FLAG
            } else {
                0
            },
            wrx: 0,
            ptr: if INLINE_SMALL {
                null_mut()
            } else {
                empty_sentinel()
            },
            offset: 0,
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
        if self.is_inlined() {
            self.len = 0 | (self.len & (INLINE_FLAG | OFFSET_MASK));
            return;
        }
        self.len = 0;
    }

    fn shrink(&mut self) {
        if self.is_inlined() {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        let target_cap = self.len + ADDITIONAL_BUFFER_CAP;
        if self.cap >= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        if !unsafe { self.is_only() } {
            // for now we just nop if there are other references to the buffer
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
            if self.is_inlined() {
                self.len = len | (self.len & (INLINE_FLAG | OFFSET_MASK));
            } else {
                self.len = len;
            }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        INLINE_SMALL && self.len & INLINE_FLAG != 0
    }

    #[inline]
    pub(crate) unsafe fn is_only(&self) -> bool {
        let meta_ptr = unsafe { self.meta_ptr() };
        unsafe { &*meta_ptr.cast::<AtomicUsize>() }.load(Ordering::Acquire) == 1
    }

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        let self_ptr = self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }>;
        if self.is_inlined() {
            if self.len() + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL>, req: usize) -> *mut u8 {
                    let offset = (unsafe { (&*buffer).len } & OFFSET_MASK) >> LEN_MASK.count_ones();
                    // remove the inline flag and offset data
                    unsafe { (&mut *buffer).len &= !(INLINE_FLAG | OFFSET_MASK); }
                    let len = unsafe { (&*buffer).len };
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, len + req + ADDITIONAL_BUFFER_CAP);

                    let alloc = unsafe { realloc_buffer_counted(buffer.cast::<u8>().add(size_of::<usize>()), len, cap) };
                    unsafe { (&mut *buffer).cap = cap };
                    unsafe { (&mut *buffer).ptr = alloc };
                    unsafe { (&mut *buffer).offset = offset };

                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(self_ptr, req);
            }
            return unsafe { self_ptr.cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req + ADDITIONAL_BUFFER_CAP {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL>, req: usize) {
                let old_cap = unsafe { (&*buffer).cap };
                let len = unsafe { (&*buffer).len };
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(old_cap, len + req + ADDITIONAL_BUFFER_CAP);
                let alloc = unsafe { realloc_buffer_and_dealloc((&*buffer).ptr, len, old_cap, new_cap) };
                unsafe { (&mut *buffer).ptr = alloc; }
                unsafe { (&mut *buffer).cap = new_cap; }
                // set ref cnt
                unsafe { *(&*buffer).meta_ptr().cast::<usize>() = 1; }
            }
            resize_alloc(self_ptr, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

    /// SAFETY: this may only be called if the buffer isn't
    /// inlined and isn't a static buffer
    #[inline]
    pub(crate) unsafe fn meta_ptr(&self) -> *mut u8 {
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(self.ptr, self.cap) }
    }

    /// SAFETY: this may only be called if the buffer is inlined.
    #[inline]
    unsafe fn inlined_buffer_ptr(&self) -> *mut u8 {
        let self_ptr = self as *const BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }>;
        unsafe { self_ptr.cast::<u8>().add(size_of::<usize>()) }.cast_mut()
    }

    #[inline]
    fn raw_offset(&self) -> usize {
        if self.is_inlined() {
            return (self.len & OFFSET_MASK) >> LEN_MASK.count_ones();
        }
        self.offset
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
WritableBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_FLAG,
                // the following two values are now treated as the buffer
                ptr: null_mut(),
                cap: 0,
                offset: 0,
            }
        } else {
            let cap = cap + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            let ret = Self {
                len: 0,
                ptr: alloc,
                cap,
                offset: 0,
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
    }

    fn zeroed(len: usize) -> Self {
        if INLINE_SMALL && len <= INLINE_SIZE {
            Self {
                len: len | INLINE_FLAG,
                // the following two values are now treated as the buffer
                ptr: null_mut(),
                cap: 0,
                offset: 0,
            }
        } else {
            let cap = len + ADDITIONAL_BUFFER_CAP;
            let alloc = alloc_zeroed_buffer(cap);
            let ret = Self {
                len,
                ptr: alloc,
                cap,
                offset: 0,
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Drop for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if !INLINE_SMALL && self.ptr == empty_sentinel() {
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
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            unsafe { dealloc(self.ptr, self.cap); }
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Clone for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_inlined() {
            return Self {
                len: self.len,
                offset: self.offset,
                ptr: self.ptr,
                cap: self.cap,
            };
        }

        let alloc = unsafe { realloc_buffer_counted(self.ptr, self.len, self.cap) };
        Self {
            len: self.len,
            ptr: alloc,
            cap: self.cap,
            offset: self.offset,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
AsRef<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr() }
        } else {
            self.ptr
        };
        unsafe { &*slice_from_raw_parts(ptr, self.len()) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Deref for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Borrow<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Default for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Into<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn into(self) -> Vec<u8> {
        if self.is_inlined() {
            let alloc = unsafe { realloc_buffer(self.inlined_buffer_ptr(), self.len(), self.len()) }; // FIXME: should we add ADDITIONAL_BUFFER_CAP?
            return unsafe { Vec::from_raw_parts(alloc, self.len(), self.len()) };
        }
        if unsafe { self.is_only() } {
            let ret = unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) };
            mem::forget(self);
            return ret;
        }
        // FIXME: should we try to shrink?
        let buf = unsafe { realloc_buffer(self.ptr, self.len, self.cap) };
        unsafe { Vec::from_raw_parts(buf, self.len, self.cap) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
From<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            // FIXME: should we instead keep the small buffer if it exists already and doesn't cost us anything?
            let mut ret = Self {
                len: len | INLINE_FLAG,
                ptr: null_mut(),
                cap: 0,
                offset: 0,
            };
            unsafe { ptr::copy_nonoverlapping(ptr, ret.inlined_buffer_ptr(), len); }
            return ret;
        }
        mem::forget(value);
        // reuse existing buffer
        let ret = Self {
            len,
            ptr,
            cap,
            offset: 0,
        };
        // set ref cnt
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        ret
    }
}
