use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::Deref;
use std::{mem, ptr};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::buffer_format::BufferFormat;
use crate::buffer_format::half::FormatHalf;
use crate::{buffer, buffer_rw, GenericBuffer, WritableBuffer};
use crate::util::{align_unaligned_ptr_to, alloc_uninit_buffer, alloc_zeroed_buffer, build_bit_mask, dealloc, empty_sentinel, find_sufficient_cap, min, realloc_buffer, realloc_buffer_and_dealloc, realloc_buffer_counted, round_up_pow_2};

pub type BufferMut = BufferMutGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

#[repr(C)]
pub struct BufferMutGeneric<LAYOUT: BufferFormat<INLINE_SMALL, false> = FormatHalf, const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true>(LAYOUT);

// FIXME: only allow cap to be a multiple of meta_align in order to be able to use the lower bits to store the additional size that was masked off to align the metadata properly

// FIXME: store base ptr and alloc_cap in the metadata in addition to the ref cnt


// TODO: additional features: allow aligning the ref cnt ptr to the cache line size

/// this is only
pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferMutGeneric<0, 0, false>>() - size_of::<usize>();
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer::BASE_INLINE_SIZE), buffer_rw::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// to align said values properly.
const ADDITIONAL_BUFFER_CAP: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Send for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {}
unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Sync for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
GenericBuffer for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_BUFFER_FLAG
            } else {
                0
            },
            buffer: BufferUnion {
                reference: ReferenceBuffer {
                    wrx: 0,
                    ptr: if INLINE_SMALL {
                        null_mut()
                    } else {
                        empty_sentinel()
                    },
                    offset: 0,
                },
            },
        }
    }

    #[inline]
    fn len(&self) -> usize {
        if self.is_inlined() {
            return self.len & INLINE_LEN_MASK;
        }
        self.len & WORD_MASK
    }

    #[inline]
    fn clear(&mut self) {
        if self.is_inlined() {
            self.len &= !INLINE_LEN_MASK;
            return;
        }
        self.len &= !WORD_MASK;
    }

    fn shrink(&mut self) {
        if self.is_inlined() {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        let target_cap = self.len + ADDITIONAL_BUFFER_CAP;
        if self.get_capacity_outlined() >= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        if !unsafe { self.is_only() } {
            // for now we just nop if there are other references to the buffer
            return;
        }
        let alloc = unsafe { realloc_buffer_counted(self.buffer.reference.ptr, self.len, target_cap) };
        let old_buf = self.buffer.reference.ptr;
        // FIXME: decrement ref cnt and only dealloc if 0
        unsafe { dealloc(old_buf, self.get_capacity_outlined()); }
        self.buffer.reference.ptr = alloc;
        self.set_capacity_outlined(target_cap);
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        if self.len() > len {
            if self.is_inlined() {
                self.len = len | (self.len & (INLINE_BUFFER_FLAG | INLINE_OFFSET_MASK));
            } else {
                self.len = len | (self.len & TAIL_MASK);
                self.buffer.reference.set_wrx(self.buffer.reference.wrx().min(len));
            }
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0
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
            if self.len() + req > INLINE_SIZE { // FIXME: check for wrx + req instead of len + req
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL>, req: usize) -> *mut u8 {
                    let offset = (unsafe { (&*buffer).len } & INLINE_OFFSET_MASK) >> INLINE_OFFSET_MASK.leading_zeros();
                    // remove the inline flag, wrx and offset data
                    let len = unsafe { (&*buffer).len } & !(INLINE_LEN_MASK | INLINE_OFFSET_MASK);
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, len + req + ADDITIONAL_BUFFER_CAP);
                    unsafe { (&mut *buffer).len = cap; }

                    let alloc = unsafe { realloc_buffer_counted(buffer.cast::<u8>().add(size_of::<usize>()), len, cap) };
                    unsafe { (&mut *buffer).set_capacity_outlined(cap) };
                    unsafe { (&mut *buffer).buffer.reference.ptr = alloc };
                    unsafe { (&mut *buffer).buffer.reference.set_offset(offset) };
                    unsafe { (&mut *buffer).buffer.reference.set_wrx(len) };

                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(self_ptr, req);
            }
            return unsafe { self_ptr.cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.len & WORD_MASK < self.buffer.reference.wrx() + req + ADDITIONAL_BUFFER_CAP {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL>, req: usize) {
                let old_cap = unsafe { (&*buffer).get_capacity_outlined() };
                let len = unsafe { (&*buffer).len } & WORD_MASK;
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(old_cap, len + req + ADDITIONAL_BUFFER_CAP);
                // FIXME: reduce ref cnt and only dealloc if refcnt is 0
                let alloc = unsafe { realloc_buffer_and_dealloc((&*buffer).buffer.reference.ptr, len, old_cap, new_cap) };
                unsafe { (&mut *buffer).buffer.reference.ptr = alloc; }
                unsafe { (&mut *buffer).set_capacity_outlined(new_cap); }
                // set ref cnt
                unsafe { *(&*buffer).meta_ptr().cast::<usize>() = 1; }
            }
            resize_alloc(self_ptr, req);
        }
        unsafe { self.buffer.reference.ptr.add(self.len & WORD_MASK) }
    }

    /// SAFETY: this may only be called if the buffer isn't
    /// inlined and isn't a static buffer
    #[inline]
    pub(crate) unsafe fn meta_ptr(&self) -> *mut u8 {
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(self.buffer.reference.ptr, self.get_capacity_outlined()) }
    }

    /// SAFETY: this may only be called if the buffer is inlined.
    #[inline]
    unsafe fn inlined_buffer_ptr(&self) -> *mut u8 {
        (&self.buffer.inlined as *const [usize; 3]).cast::<u8>().cast_mut()
    }

    #[inline]
    fn raw_offset(&self) -> usize {
        if self.is_inlined() {
            return (self.len & INLINE_OFFSET_MASK) >> INLINE_OFFSET_MASK.leading_zeros();
        }
        self.buffer.reference.offset()
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
WritableBuffer for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_BUFFER_FLAG,
                // the following two values are now treated as the buffer
                buffer: BufferUnion { inlined: [0; 3] },
            }
        } else {
            let cap = cap + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            let mut len = 0;
            let buffer = ReferenceBuffer::new(&mut len, cap, 0, 0, alloc);
            let ret = Self {
                len,
                buffer: BufferUnion { reference: buffer },
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
                buffer: BufferUnion { inlined: [0; 3], },
            }
        } else {
            let cap = len + ADDITIONAL_BUFFER_CAP;
            let alloc = alloc_zeroed_buffer(cap);
            let mut len = 0;
            let buffer = ReferenceBuffer::new(&mut len, cap, 0, 0, alloc);
            let ret = Self {
                len,
                buffer: BufferUnion { reference: buffer },
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
        self.get_capacity_outlined()
    }

    #[inline]
    fn put_slice(&mut self, val: &[u8]) {
        let ptr = self.ensure_large_enough(val.len());
        unsafe { ptr::copy_nonoverlapping(val as *const [u8] as *const u8, ptr, val.len()); }
        self.len += val.len();
    }

    fn put_bytes(&mut self, val: u8, repeat: usize) {
        let ptr = self.ensure_large_enough(repeat);
        unsafe { ptr::write_bytes(ptr, val, repeat); }
        self.len += repeat;
    }

    #[inline]
    fn put_u8(&mut self, val: u8) {
        let ptr = self.ensure_large_enough(1);
        unsafe { *ptr = val; }
        self.len += 1;
    }

    fn reserve(&mut self, size: usize) {
        self.ensure_large_enough(size);
    }

    fn resize(&mut self, size: usize) {
        
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Drop for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if !INLINE_SMALL && self.buffer.reference.ptr == empty_sentinel() {
            // we don't do anything for empty buffers
            return;
        }
        // fast path for single ref cnt scenarios
        if unsafe { self.is_only() } {
            unsafe { dealloc(self.buffer.reference.ptr, self.get_capacity_outlined()); }
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            unsafe { dealloc(self.buffer.reference.ptr, self.capacity()); }
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Clone for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_inlined() {
            return Self {
                len: self.len,
                buffer: BufferUnion { inlined: self.buffer.inlined, },
            };
        }

        let alloc = unsafe { realloc_buffer_counted(self.buffer.reference.ptr, self.len & WORD_MASK, self.get_capacity_outlined()) };
        let mut len = self.len;
        let mut reference = ReferenceBuffer::new(&mut len, self.get_capacity_outlined(), self.buffer.reference.offset(), self.buffer.reference.wrx(), alloc);
        
        Self {
            len: self.len,
            buffer: BufferUnion { reference, },
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
AsRef<[u8]> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr() }
        } else {
            self.buffer.reference.ptr
        };
        unsafe { &*slice_from_raw_parts(ptr, self.len()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Deref for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Borrow<[u8]> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Default for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
Into<Vec<u8>> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    #[inline]
    fn into(self) -> Vec<u8> {
        if self.is_inlined() {
            let alloc = unsafe { realloc_buffer(self.inlined_buffer_ptr(), self.len(), self.len()) }; // FIXME: should we add ADDITIONAL_BUFFER_CAP?
            return unsafe { Vec::from_raw_parts(alloc, self.len(), self.len()) };
        }
        let len = self.len & WORD_MASK;
        if unsafe { self.is_only() } {
            let ret = unsafe { Vec::from_raw_parts(self.buffer.reference.ptr, len, self.get_capacity_outlined()) };
            mem::forget(self);
            return ret;
        }
        // FIXME: should we try to shrink?
        let buf = unsafe { realloc_buffer(self.buffer.reference.ptr, len, self.get_capacity_outlined()) };
        unsafe { Vec::from_raw_parts(buf, len, self.get_capacity_outlined()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool>
From<Vec<u8>> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL> {
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let mut len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            // FIXME: should we instead keep the small buffer if it exists already and doesn't cost us anything?
            let mut ret = Self {
                len: len | INLINE_BUFFER_FLAG,
                buffer: BufferUnion { inlined: [0; 3], },
            };
            unsafe { ptr::copy_nonoverlapping(ptr, ret.inlined_buffer_ptr(), len); }
            return ret;
        }
        mem::forget(value);
        let buffer = ReferenceBuffer::new(&mut len, cap, 0, 0, ptr);
        // reuse existing buffer
        let ret = Self {
            len,
            buffer: BufferUnion { reference: buffer, },
        };
        // set ref cnt
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        ret
    }
}
