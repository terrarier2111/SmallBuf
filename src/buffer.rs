use std::mem::{align_of, size_of, transmute};
use std::ops::{Deref, RangeBounds};
use std::process::abort;
use std::{mem, ptr};
use std::borrow::Borrow;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::buffer_format::{BufferFormat, Flags};
use crate::buffer_format::half::FormatHalf;
use crate::{buffer_mut, buffer_rw, GenericBuffer, ReadableBuffer, ReadonlyBuffer, WritableBuffer};
use crate::buffer_mut::BufferMutGeneric;
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, build_bit_mask, dealloc, empty_sentinel, min, realloc_buffer, realloc_buffer_counted};

pub type Buffer = BufferGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

#[repr(C)]
pub struct BufferGeneric<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE> = FormatHalf, const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true>(LAYOUT);

pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferGeneric<0, 0, false, false>>() - size_of::<usize>();
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer_mut::BASE_INLINE_SIZE), buffer_rw::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// to align said values properly.
const ADDITIONAL_BUFFER_CAP: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Send for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}
unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Sync for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

    #[inline]
    pub(crate) fn is_static(&self) -> bool {
        STATIC_STORAGE && self.0.flags().is_static_reference()
    }

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        INLINE_SMALL && self.0.flags().is_inlined()
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
        let rdx = self.get_rdx();
        let offset = self.get_offset();
        if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr().add(offset + rdx) }
        } else {
            unsafe { self.0.ptr_reference().add(offset + rdx) }
        }
    }

    #[inline]
    pub(crate) fn get_offset(&self) -> usize {
        if self.is_inlined() {
            return (self.len & INLINE_OFFSET_MASK) >> INLINE_OFFSET_MASK.leading_zeros();
        }
        self.buffer.reference.offset()
    }

    #[inline]
    pub(crate) fn set_offset(&self, offset: usize) {
        if self.is_inlined() {
            self.len = offset | (self.len & (INLINE_LEN_MASK | BUFFER_TY_MASK));
            return;
        }
        self.buffer.reference.set_offset(offset);
    }

    #[inline]
    pub(crate) fn get_rdx(&self) -> usize {
        if self.is_inlined() {
            return (self.len & RDX_MASK) >> LEN_MASK.leading_zeros();
        }
        self.buffer.reference.rdx()
    }

    #[inline]
    fn set_rdx(&mut self, rdx: usize) {
        if self.is_inlined() {
            self.len = (self.len & !RDX_MASK) | (rdx << LEN_MASK.count_ones());
            return;
        }
        self.buffer.reference.set_rdx(rdx);
    }

    #[inline]
    fn get_len(&self) -> usize {
        if !STATIC_STORAGE && !INLINE_SMALL {
            return self.len;
        }
        /*let zero_or_mask = 0 - ((self.len & INLINE_BUFFER_FLAG) >> INLINE_BUFFER_FLAG.leading_zeros());
        let conv = !(zero_or_mask & !(BUFFER_TY_MASK | LEN_MASK));
        let inlined_bit_mask = (usize::MAX & !BUFFER_TY_MASK) & conv;
        inlined_bit_mask*/
        if self.is_inlined() {
            return self.len & LEN_MASK;
        }
        self.len & !BUFFER_TY_MASK
    }

    #[inline]
    fn set_len(&mut self, len: usize) {
        let flags = if INLINE_SMALL {
            self.len & (BUFFER_TY_MASK | RDX_MASK)
        } else {
            0
        };
        self.len = len | flags;
    }

    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if self.is_inlined() {
            return INLINE_SIZE;
        }
        self.get_capacity_outlined()
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
        self.0.ptr_inlined()
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
GenericBuffer for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn new() -> Self {
        if !INLINE_SMALL && STATIC_STORAGE {
            static EMPTY: &[u8] = &[];
            return Self::from(EMPTY);
        }

        if INLINE_SMALL {
            Self(LAYOUT::new_inlined(INLINE_SIZE, 0, [0; 3]))
        } else {
            Self(LAYOUT::new_reference(0, 0, 0, 0, 0, empty_sentinel(), LAYOUT::FlagsTy::new_reference()))
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.get_len() - self.get_offset()
    }

    #[inline]
    fn clear(&mut self) {
        let _ = mem::replace(self, BufferGeneric::new());
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
        let target_cap = self.0.len_reference() + ADDITIONAL_BUFFER_CAP;
        if self.0.cap_reference() <= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        let old = self.0.ptr_reference();
        let alloc = unsafe { realloc_buffer_counted(old, self.0.len_reference(), target_cap) };
        let cap = self.0.cap_reference();
        unsafe { dealloc(old, cap); }
        self.0.set_ptr_reference(alloc);
        self.0.set_cap_reference(self.0.len_reference());
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

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
ReadableBuffer for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {

    #[inline]
    fn remaining(&self) -> usize {
        self.len() - self.get_rdx()
    }

    #[inline]
    fn split_off(&mut self, offset: usize) -> Self {
        let idx = self.get_offset() + self.get_rdx() + offset;
        assert!(self.get_len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        self.set_len(idx);
        other.set_offset(idx);

        other
    }

    #[inline]
    fn split_to(&mut self, offset: usize) -> Self {
        let idx = self.get_offset() + self.get_rdx() + offset;
        assert!(self.get_len() > idx, "tried splitting buffer with length {} at {}", self.len, idx);
        let mut other = self.clone();

        other.set_len(idx);
        self.set_offset(idx);

        other
    }

    #[inline]
    fn split(&mut self) -> Self {
        // TODO: check if the panic check gets removed
        self.split_off(0)
    }

    fn unsplit(&mut self, other: Self) {
        // FIXME: handle offsets in this method correctly and fix usage of `self.len`!
        if self.is_empty() {
            *self = other;
            return;
        }

        if self.is_static() {
            if !other.is_static() {
                panic!("Static buffers can only be merged with other static buffers");
            }
            if self.buffer.reference.ptr != other.buffer.reference.ptr {
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

        if self.buffer.reference.ptr != other.buffer.reference.ptr {
            panic!("Unsplitting only works on buffers that have the same src");
        }
        if self.get_rdx() + self.len() != other.get_rdx() && self.get_rdx() != other.get_rdx() + other.len() {
            panic!("Unsplitting only works on buffers that are next to each other");
        }
        self.set_rdx(self.get_rdx().min(other.get_rdx()));
        self.set_len(self.len().max(other.len()));
    }

    #[inline]
    fn get_slice(&mut self, bytes: usize) -> &[u8] {
        let ptr = self.ensure_readable(bytes);
        self.buffer.reference.rdx += bytes;
        unsafe { &*slice_from_raw_parts(ptr, bytes) }
    }

    #[inline]
    fn get_u8(&mut self) -> u8 {
        let ptr = self.ensure_readable(1);
        self.buffer.reference.rdx += 1;
        unsafe { *ptr }
    }

    fn advance(&mut self, amount: usize) {
        let new_rdx = self.get_rdx() + amount;
        assert!(new_rdx < self.len());
        self.set_rdx(new_rdx);
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
ReadonlyBuffer for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn slice(&self, range_offset: impl RangeBounds<usize>) -> Self {
        todo!()
    }
}

const MAX_REF_CNT: usize = usize::MAX / 2;

#[inline]
fn increment_ref_cnt(ref_cnt: &AtomicUsize) {
    let val = ref_cnt.fetch_add(1, Ordering::AcqRel); // FIXME: can we choose a weaker ordering?
    if val > MAX_REF_CNT {
        abort();
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Drop for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if self.is_static() {
            // we don't need to do anything for static buffers
            return;
        }
        if !INLINE_SMALL && !STATIC_STORAGE && self.0.ptr_reference() == empty_sentinel() {
            // we don't do anything for empty buffers
            return;
        }
        // fast path for single ref cnt scenarios
        if unsafe { self.is_only() } {
            unsafe { dealloc(self.buffer.reference.ptr, self.buffer.reference.capacity()); }
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            let cap = self.capacity();
            unsafe { dealloc(self.buffer.reference.ptr.cast::<u8>(), cap); }
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Clone for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn clone(&self) -> Self {
        if !self.is_inlined() && !self.is_static() {
            let meta_ptr = unsafe { self.meta_ptr() };
            // increase the ref cnt if the buffer isn't inlined
            increment_ref_cnt(unsafe { &*meta_ptr.cast::<AtomicUsize>() });
        }
        Self {
            len: self.len,
            buffer: BufferUnion { inlined: self.buffer.inlined },
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
AsRef<[u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { self.inlined_buffer_ptr() }
        } else {
            self.buffer.reference.ptr
        };
        let rdx = self.get_rdx();
        let ptr = unsafe { ptr.add(rdx) };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Deref for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Borrow<[u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Default for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<&'static [u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        let mut len = value.len();
        let buffer = ReferenceBuffer::new(&mut len, value.len(), 0, 0, value as *const [u8] as *mut u8);
        Self {
            len,
            buffer: BufferUnion { reference: buffer, },
            
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<Vec<u8>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> Vec<u8> {
        // handle inlined buffers
        if self.is_inlined() {
            return Vec::from(unsafe { &*slice_from_raw_parts(self.inlined_buffer_ptr(), self.len()) });
        }

        if self.is_static() {
            // try reusing buffer
            if unsafe { self.is_only() } {
                let ret = unsafe { Vec::from_raw_parts(self.buffer.reference.ptr, self.len & WORD_MASK, self.get_capacity_outlined()) };
                mem::forget(self);
                return ret;
            }
        }
        // TODO: should we try to shrink?
        let cap = self.get_capacity_outlined();
        let alloc = unsafe { realloc_buffer(self.buffer.reference.ptr, self.len & WORD_MASK, cap) };
        unsafe { Vec::from_raw_parts(alloc, cap, self.len & WORD_MASK) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<Vec<u8>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn from(mut value: Vec<u8>) -> Self {
        let mut ptr = value.as_mut_ptr();
        let mut cap = value.capacity();
        let mut len = value.len();
        let available = cap - len;
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            // FIXME: should we reuse the buffer if possible?
            let mut ret = Self {
                len: len | INLINE_BUFFER_FLAG,
                buffer: BufferUnion { inlined: [0; 3] },
            };
            unsafe { ptr::copy_nonoverlapping(ptr, ret.inlined_buffer_ptr(), len); }
            return ret;
        }
        // try reusing existing buffer
        let ref_cnt_ptr = if available < ADDITIONAL_BUFFER_CAP {
            cap = len + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { realloc_buffer(ptr, len, cap) };
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(alloc, cap) };
            ptr = alloc;
            aligned
        } else {
            mem::forget(value);
            let aligned = unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(ptr, cap) };
            aligned
        };
        // init ref cnt
        unsafe { *ref_cnt_ptr.cast::<usize>() = 1; }
        let buffer = ReferenceBuffer::new(&mut len, cap, 0, 0, ptr);
        Self {
            len,
            buffer: BufferUnion { reference: buffer, },
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    fn from(value: BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>) -> Self {
        if value.is_inlined() {
            return Self {
                len: value.len,
                buffer: BufferUnion { inlined: value.buffer.inlined, },
            };
        }
        let aligned_len = align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(value.buffer.reference.ptr, value.len) + size_of::<AtomicUsize>();
        debug_assert_eq!((value.buffer.reference.ptr as usize + aligned_len) % align_of::<AtomicUsize>(), 0);
        // reuse the buffer if this instance as we know that we are the only reference to this part of the buffer
        unsafe { transmute::<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>, BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE>>(value) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE> {
    #[inline]
    fn into(self) -> BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL> {
        if self.is_inlined() {
            return BufferMutGeneric {
                len: self.len,
                buffer: buffer_mut::BufferUnion { inlined: self.buffer.inlined, },
            };
        }
        if self.is_static() {
            let cap = self.len + ADDITIONAL_BUFFER_CAP; // TODO: should we multiply with growth factor?
            let alloc = unsafe { realloc_buffer(self.buffer.reference.ptr, self.len & WORD_MASK, cap) };
            let mut len = self.len & WORD_MASK;
            let buffer = buffer_mut::ReferenceBuffer::new(&mut len, cap, self.get_offset(), len, alloc);
            let ret = BufferMutGeneric {
                len,
                buffer: buffer_mut::BufferUnion { reference: buffer, },
            };
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            return ret;
        }
        if unsafe { self.is_only() } {
            let ret = BufferMutGeneric {
                len: self.len,
                ptr: self.ptr,
                cap: self.cap,
                offset: self.rdx,
            };
            mem::forget(self);
            return ret;
        }
        let alloc = unsafe { realloc_buffer_counted(self.buffer.reference.ptr, self.len & WORD_MASK, (self.len & WORD_MASK) + ADDITIONAL_BUFFER_CAP) };

        BufferMutGeneric {
            len: self.len,
            ptr: alloc,
            cap: self.len + ADDITIONAL_BUFFER_CAP,
            offset: self.rdx,
        }
    }
}
