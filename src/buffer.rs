use std::mem::{align_of, size_of, transmute};
use std::ops::{Deref, RangeBounds};
use std::process::abort;
use std::{mem, ptr};
use std::borrow::Borrow;
use std::ptr::slice_from_raw_parts;
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::buffer_format::{BufferFormat, Flags};
use crate::buffer_format::half::FormatHalf;
use crate::{buffer_mut, buffer_rw, GenericBuffer, ReadableBuffer, ReadonlyBuffer, WritableBuffer};
use crate::buffer_mut::BufferMutGeneric;
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, dealloc, empty_sentinel, min, realloc_buffer, realloc_buffer_counted};

pub type Buffer = BufferGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

#[repr(C)]
pub struct BufferGeneric<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE> = FormatHalf, const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true, const RETAIN_INDICES: bool = true>(pub(crate) LAYOUT);

pub(crate) const BASE_INLINE_SIZE: usize = size_of::<BufferGeneric<0, 0, false, false>>() - size_of::<usize>();
const INLINE_SIZE: usize = min(min(BASE_INLINE_SIZE, buffer_mut::BASE_INLINE_SIZE), buffer_rw::BASE_INLINE_SIZE);
/// this additional storage is used to store the reference counter and
/// to align said values properly.
const ADDITIONAL_BUFFER_CAP: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Send for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {}
unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Sync for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {

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
        unsafe { self.0.ptr().add(self.0.offset() + self.0.rdx()) }
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
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(self.0.ptr_reference(), self.get_capacity_outlined()) }
    }

    #[inline]
    unsafe fn increment_ref_cnt(&self) {
        unsafe { &*self.meta_ptr().cast::<AtomicUsize>() }.fetch_add(1, Ordering::AcqRel);
    }

    #[inline]
    unsafe fn decrement_ref_cnt(&self) -> usize {
        unsafe { &*self.meta_ptr().cast::<AtomicUsize>() }.fetch_sub(1, Ordering::AcqRel) - 1
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
        self.0.wrx()
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
        let alloc = unsafe { realloc_buffer_counted(old, self.0.offset_reference(), self.0.len_reference(), target_cap) };
        let cap = self.0.cap_reference();
        if unsafe { self.decrement_ref_cnt() } == 0 {
            unsafe { dealloc(old, cap); }
        }
        self.0.set_ptr_reference(alloc);
        self.0.set_cap_reference(self.0.len_reference());
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        if self.0.len() > len {
            self.0.set_len(len);
            // fixup rdx and wrx after updating len in order to avoid the rdx getting OOB
            self.0.set_rdx(self.0.rdx().min(len));
            self.0.set_wrx(self.0.wrx().min(len));
        }
    }

    #[inline]
    fn split_off(&mut self, offset: usize) -> Self {
        let idx = self.0.offset() + self.0.rdx() + offset;
        assert!(self.len() > idx, "tried splitting buffer with length {} at {}", self.0.len(), idx);
        let mut other = self.clone();

        self.0.set_len(idx);
        other.0.set_offset(idx);

        other
    }

    #[inline]
    fn split_to(&mut self, offset: usize) -> Self {
        let other = if self.is_inlined() {
            let mut other = Self(LAYOUT::new_inlined(self.0.len_inlined() + offset, self.0.offset_inlined(), unsafe { *self.0.ptr_inlined().cast::<[usize; 3]>() }));
            other.0.set_wrx_inlined(self.0.wrx_inlined());
            other.0.set_rdx_inlined(self.0.rdx_inlined());
            other
        } else {
            unsafe { self.increment_ref_cnt(); }
            let other = Self(LAYOUT::new_reference(self.0.wrx_reference() + offset, self.0.cap_reference(), self.0.wrx_reference(), self.0.rdx_reference(), self.0.offset_reference(), self.0.ptr_reference(), self.0.flags()));
            other
        };
        self.0.set_len(self.0.wrx() + offset);
        self.0.set_rdx(0);
        self.0.set_offset(self.0.offset() + self.0.wrx() + offset);
        self.0.set_wrx(0);
        other
    }

    fn split(&mut self) -> Self {
        self.split_off(0)
    }

    fn unsplit(&mut self, other: Self) {
        self.try_unsplit(other).unwrap();
    }

    fn try_unsplit(&mut self, other: Self) -> Result<(), Self> {
        if self.0.flags() != other.0.flags() {
            return Err(other);
        }
        let own_off = self.0.offset();
        let other_off = other.0.offset();
        let (min, max) = if own_off < other_off {
            (&self, &other)
        } else {
            (&other, &self)
        };

        // check if the left buffer still has uninit data
        if min.0.wrx() != min.0.len() {
            return Err(other);
        }

        let dist = max.0.offset() - min.0.offset();
        // check if buffers are adjacent
        if dist != min.0.wrx() {
            return Err(other);
        }

        // check if ptrs aren't matching
        if !self.0.flags().is_inlined() && min.0.ptr_reference() != max.0.ptr_reference() {
            return Err(other);
        }

        self.0.set_len_inlined(self.0.len() + other.0.len());
        self.0.set_offset(self.0.offset().min(other.0.offset()));
        self.0.set_wrx(self.0.wrx() + other.0.wrx());
        self.0.set_rdx(0);
        Ok(())
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
ReadableBuffer for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {

    #[inline]
    fn remaining(&self) -> usize {
        self.0.len() - self.0.rdx()
    }

    #[inline]
    fn get_slice(&mut self, bytes: usize) -> &[u8] {
        let ptr = self.ensure_readable(bytes);
        self.0.set_rdx(self.0.rdx() + bytes);
        unsafe { &*slice_from_raw_parts(ptr, bytes) }
    }

    #[inline]
    fn get_u8(&mut self) -> u8 {
        let ptr = self.ensure_readable(1);
        self.0.set_rdx(self.0.rdx() + 1);
        unsafe { *ptr }
    }

    fn advance(&mut self, amount: usize) {
        let new_rdx = self.0.rdx() + amount;
        assert!(new_rdx < self.0.len());
        self.0.set_rdx(new_rdx);
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
ReadonlyBuffer for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
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

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Drop for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
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
            unsafe { dealloc(self.buffer.reference.ptr, self.0.cap_reference()); }
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            let cap = self.0.cap_reference();
            unsafe { dealloc(self.0.ptr_reference().cast::<u8>(), cap); }
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Clone for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn clone(&self) -> Self {
        if !self.is_inlined() && !self.is_static() {
            let meta_ptr = unsafe { self.meta_ptr() };
            // increase the ref cnt if the buffer isn't inlined
            increment_ref_cnt(unsafe { &*meta_ptr.cast::<AtomicUsize>() });
        }
        Self(self.0.clone())
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
AsRef<[u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = self.0.ptr();
        let rdx = self.0.rdx();
        let ptr = unsafe { ptr.add(rdx) };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Deref for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Borrow<[u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Default for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
From<&'static [u8]> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self(LAYOUT::new_reference(value.len(), value.len(), 0, 0, 0, value as *const [u8] as *mut u8, LAYOUT::FlagsTy::new_static_reference()))
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
Into<Vec<u8>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn into(self) -> Vec<u8> {
        // FIXME: should we add ADDITIONAL_BUFFER_CAP on realloc?

        // handle inlined buffers
        if self.is_inlined() {
            let alloc = unsafe { realloc_buffer(self.0.ptr_inlined(), self.0.offset_inlined(), self.0.len_inlined(), self.0.len_inlined()) };
            return unsafe { Vec::from_raw_parts(alloc, self.0.len(), self.0.len()) };
        }
        let len = self.0.len_reference();
        // try reusing buffer
        if unsafe { self.is_only() } && self.0.offset_reference() == 0 {
            let ret = unsafe { Vec::from_raw_parts(self.0.ptr_reference(), len, self.0.cap_reference()) };
            mem::forget(self);
            return ret;
        }
        // FIXME: should we try to shrink?
        let buf = unsafe { realloc_buffer(self.0.ptr_reference(), self.0.offset_reference(), len, self.0.cap_reference()) };
        unsafe { Vec::from_raw_parts(buf, len, self.0.cap_reference()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const RETAIN_INDICES: bool>
From<Vec<u8>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let mut len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            // FIXME: should we instead keep the small buffer if it exists already and doesn't cost us anything?
            let mut ret = Self(LAYOUT::new_inlined(len, 0, [0; 3]));
            unsafe { ptr::copy_nonoverlapping(ptr, ret.0.ptr_inlined(), len); }
            return ret;
        }
        mem::forget(value);
        // reuse existing buffer
        let ret = Self(LAYOUT::new_reference(len, cap, 0, 0, 0, ptr, LAYOUT::FlagsTy::new_reference()));
        // set ref cnt
        unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
        ret
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const RETAIN_INDICES_OTHER: bool, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const RETAIN_INDICES: bool, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
From<BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, RETAIN_INDICES_OTHER>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    fn from(value: BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, RETAIN_INDICES_OTHER>) -> Self {
        if value.is_inlined() {
            return Self(value.0.clone());
        }
        let aligned_len = align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(value.0.ptr_reference(), value.len) + size_of::<AtomicUsize>();
        debug_assert_eq!((value.0.ptr_reference() as usize + aligned_len) % align_of::<AtomicUsize>(), 0);
        // reuse the buffer if this instance as we know that we are the only reference to this part of the buffer
        unsafe { transmute::<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL>, BufferGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE>>(value) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, STATIC_STORAGE>, const GROWTH_FACTOR_OTHER: usize, const RETAIN_INDICES_OTHER: bool, const INITIAL_CAP_OTHER: usize, const GROWTH_FACTOR: usize, const RETAIN_INDICES: bool, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool>
Into<BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, RETAIN_INDICES_OTHER>> for BufferGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, RETAIN_INDICES> {
    #[inline]
    fn into(self) -> BufferMutGeneric<LAYOUT, GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, RETAIN_INDICES_OTHER> {
        if self.is_inlined() {
            return BufferMutGeneric(self.0);
        }
        if self.is_static() {
             // TODO: should we multiply with growth factor?
            let alloc = unsafe { realloc_buffer_counted(self.0.ptr_reference(), self.0.offset_reference(), self.0.len_reference(), self.0.len_reference() + ADDITIONAL_BUFFER_CAP) };
            return BufferMutGeneric(LAYOUT::new_reference(self.0.len_reference(), self.0.len_reference() + ADDITIONAL_BUFFER_CAP, self.0.wrx_reference(), self.0.rdx_reference(), self.0.offset_reference(), self.0.ptr_reference(), self.0.flags()));
        }
        if unsafe { self.is_only() } {
            let ret = BufferMutGeneric(self.0);
            mem::forget(self);
            return ret;
        }
        let alloc = unsafe { realloc_buffer_counted(self.0.ptr_reference(), self.0.offset_reference(), self.0.len_reference(), self.0.cap_reference()) };

        BufferMutGeneric(LAYOUT::new_reference(self.0.len_reference(), self.0.cap_reference(), self.0.wrx_reference(), self.0.rdx_reference(), self.0.offset_reference(), self.0.ptr_reference(), self.0.flags()))
    }
}
