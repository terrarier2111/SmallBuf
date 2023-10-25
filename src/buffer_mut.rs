use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::Deref;
use std::{mem, ptr};
use std::ptr::slice_from_raw_parts;
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::buffer_format::{BufferFormat, Flags};
use crate::buffer_format::half::FormatHalf;
use crate::{buffer, buffer_rw, GenericBuffer, WritableBuffer};
use crate::util::{align_unaligned_ptr_to, alloc_uninit_buffer, alloc_zeroed_buffer, dealloc, empty_sentinel, find_sufficient_cap, min, realloc_buffer, realloc_buffer_and_dealloc, realloc_buffer_counted, round_up_pow_2};

pub type BufferMut = BufferMutGeneric;

// TODO: once const_generic_expressions are supported calculate INITIAL_CAP the following:
// INITIAL_CAP = GROWTH_FACTOR * INLINE_SIZE
const INITIAL_CAP_DEFAULT: usize = (2 * INLINE_SIZE).next_power_of_two();

pub struct BufferMutGeneric<LAYOUT: BufferFormat<INLINE_SMALL, false> = FormatHalf, const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = INITIAL_CAP_DEFAULT, const INLINE_SMALL: bool = true, const RETAIN_INDICES: bool = true>(LAYOUT);

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

unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Send for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {}
unsafe impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Sync for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
GenericBuffer for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn new() -> Self {
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
        self.0.set_rdx(0);
        self.0.set_wrx(0);
    }

    fn shrink(&mut self) {
        if self.is_inlined() {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        let target_cap = self.0.wrx_reference() + ADDITIONAL_BUFFER_CAP;
        if self.0.len_reference() <= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        if !unsafe { self.is_only() } {
            // for now we just nop if there are other references to the buffer
            return;
        }
        let old_buf = self.0.ptr_reference();
        let alloc = unsafe { realloc_buffer_counted(old_buf, self.0.offset_reference(), self.0.wrx_reference(), target_cap) };
        // FIXME: decrement ref cnt and only dealloc if 0
        unsafe { dealloc(old_buf, self.0.cap_reference()); }
        self.0.set_ptr_reference(alloc);
        self.0.set_cap_reference(target_cap);
        self.0.set_len_reference(self.0.wrx_reference());
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        if self.len() > len {
            if self.is_inlined() {
                self.0.set_rdx_inlined(self.0.rdx_inlined().min(len));
                self.0.set_wrx_inlined(self.0.wrx_inlined().min(len));
            } else {
                self.0.set_rdx_reference(self.0.rdx_reference().min(len));
                self.0.set_wrx_reference(self.0.wrx_reference().min(len));
            }
        }
    }

    fn split_off(&mut self, offset: usize) -> Self {
        
    }

    fn split_to(&mut self, offset: usize) -> Self {
        
    }

    fn split(&mut self) -> Self {
        
    }

    fn unsplit(&mut self, other: Self) {
        
    }

    fn try_unsplit(&mut self, other: Self) -> Result<(), Self> {
        
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        self.0.flags().is_inlined()
    }

    #[inline]
    pub(crate) unsafe fn is_only(&self) -> bool {
        let meta_ptr = unsafe { self.meta_ptr() };
        unsafe { &*meta_ptr.cast::<AtomicUsize>() }.load(Ordering::Acquire) == 1
    }

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        let self_ptr = self as *mut BufferMutGeneric<LAYOUT, { GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { RETAIN_INDICES }>;
        if self.is_inlined() {
            if self.len() + req > INLINE_SIZE { // FIXME: check for wrx + req instead of len + req
                #[cold]
                #[inline(never)]
                fn outline_buffer<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>(buffer: *mut BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES>, req: usize) -> *mut u8 {
                    let offset = unsafe { (&*buffer).0.offset_inlined() };
                    // remove the inline flag, wrx and offset data
                    let len = unsafe { (&*buffer).0.len_inlined() };
                    let rdx = unsafe { (&*buffer).0.rdx_inlined() };
                    let wrx = unsafe { (&*buffer).0.wrx_inlined() };
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, offset + wrx + req + ADDITIONAL_BUFFER_CAP);

                    let alloc = unsafe { realloc_buffer_counted((&*buffer).0.ptr_inlined(), offset, wrx, cap) };
                    unsafe { (&mut *buffer).0.set_cap_reference(cap) };
                    unsafe { (&mut *buffer).0.set_len_reference(cap - ADDITIONAL_BUFFER_CAP - offset) };
                    unsafe { (&mut *buffer).0.set_ptr_reference(alloc) };
                    unsafe { (&mut *buffer).0.set_offset_reference(offset) };
                    unsafe { (&mut *buffer).0.set_wrx_reference(wrx) };
                    unsafe { (&mut *buffer).0.set_rdx_reference(rdx) };

                    unsafe { alloc.add(wrx) }
                }
                // handle outlining buffer
                return outline_buffer(self_ptr, req);
            }
            return unsafe { (&*self_ptr).0.ptr_inlined().add(self.0.offset_inlined() + self.0.wrx_inlined()) };
        }
        // handle buffer reallocation
        if self.0.len_reference() < self.0.wrx_reference() + req + ADDITIONAL_BUFFER_CAP {
            #[inline(never)]
            #[cold]
            fn resize_alloc<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICIES: bool>(buffer: *mut BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICIES>, req: usize) {
                let old_cap = unsafe { (&*buffer).0.cap_reference() };
                let len = unsafe { (&*buffer).0.len_reference() };
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(round_up_pow_2(len).min(old_cap), len + req + ADDITIONAL_BUFFER_CAP);
                // FIXME: reduce ref cnt and only dealloc if refcnt is 0
                let alloc = unsafe { realloc_buffer_and_dealloc((&*buffer).0.ptr_reference(), (&*buffer).0.offset_reference(), len, old_cap, new_cap) };
                unsafe { (&mut *buffer).0.set_ptr_reference(alloc); }
                unsafe { (&mut *buffer).0.set_cap_reference(new_cap); }
                unsafe { (&mut *buffer).0.set_len_reference(new_cap); }
                unsafe { (&mut *buffer).0.set_offset_reference(0); }
                // set ref cnt
                unsafe { *(&*buffer).meta_ptr().cast::<usize>() = 1; }
            }
            resize_alloc(self_ptr, req);
        }
        unsafe { self.0.ptr_reference().add(self.0.offset_reference() + self.0.wrx_reference()) }
    }

    /// SAFETY: this may only be called if the buffer isn't
    /// inlined and isn't a static buffer
    #[inline]
    pub(crate) unsafe fn meta_ptr(&self) -> *mut u8 {
        unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(self.0.ptr_reference(), self.0.cap_reference()) }
    }

}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
WritableBuffer for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self(LAYOUT::new_inlined(INLINE_SIZE, 0, [0; 3]))
        } else {
            let cap = cap + ADDITIONAL_BUFFER_CAP;
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            let ret = Self(LAYOUT::new_reference(0, cap, 0, 0, 0, alloc, LAYOUT::FlagsTy::new_reference()));
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
    }

    fn zeroed(len: usize) -> Self {
        if INLINE_SMALL && len <= INLINE_SIZE {
            Self(LAYOUT::new_inlined(INLINE_SIZE, 0, [0; 3])) // FIXME: pass wrx (and maybe rdx) to inlined constructor
        } else {
            let cap = len + ADDITIONAL_BUFFER_CAP;
            let alloc = alloc_zeroed_buffer(cap);
            let ret = Self(LAYOUT::new_reference(len, cap, 0, 0, 0, alloc, LAYOUT::FlagsTy::new_reference()));
            // set ref cnt
            unsafe { *ret.meta_ptr().cast::<usize>() = 1; }
            ret
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        // we treat the len as our cap as that's what is effectively usable for the buffer's user
        self.0.len()
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

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool, const RETAIN_INDICES: bool>
Drop for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if !INLINE_SMALL && self.0.ptr_reference() == empty_sentinel() {
            // we don't do anything for empty buffers
            return;
        }
        // fast path for single ref cnt scenarios
        if unsafe { self.is_only() } {
            unsafe { dealloc(self.0.ptr_reference(), self.0.cap_reference()); }
            return;
        }
        let meta_ptr = unsafe { self.meta_ptr() };
        let ref_cnt = unsafe { &*meta_ptr.cast::<AtomicUsize>() };
        let remaining = ref_cnt.fetch_sub(1, Ordering::AcqRel) - 1; // FIXME: can we choose a weaker ordering?
        if remaining == 0 {
            unsafe { dealloc(self.0.ptr_reference(), self.0.cap_reference()); }
        }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Clone for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_inlined() {
            return Self(self.0.clone());
        }

        // FIXME: support offset in realloc!
        let alloc = unsafe { realloc_buffer_counted(self.0.ptr_reference(), self.0.offset_reference(), self.0.wrx_reference(), self.0.cap_reference()) };
        
        Self(LAYOUT::new_reference(self.0.len_reference(), self.0.cap_reference(), self.0.wrx_reference(), self.0.rdx_reference(), self.0.offset_reference(), alloc, self.0.flags()))
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
AsRef<[u8]> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        unsafe { &*slice_from_raw_parts(self.0.ptr(), self.len()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Deref for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Borrow<[u8]> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Default for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
Into<Vec<u8>> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
    #[inline]
    fn into(self) -> Vec<u8> {
        if self.is_inlined() {
            let alloc = unsafe { realloc_buffer(self.0.ptr_inlined(), self.0.len(), self.0.len()) }; // FIXME: should we add ADDITIONAL_BUFFER_CAP?
            return unsafe { Vec::from_raw_parts(alloc, self.0.len(), self.0.len()) };
        }
        let len = self.0.len_reference();
        if unsafe { self.is_only() } {
            let ret = unsafe { Vec::from_raw_parts(self.0.ptr_reference(), len, self.0.cap_reference()) };
            mem::forget(self);
            return ret;
        }
        // FIXME: should we try to shrink?
        let buf = unsafe { realloc_buffer(self.0.ptr_reference(), len, self.0.cap_reference()) };
        unsafe { Vec::from_raw_parts(buf, len, self.0.cap_reference()) }
    }
}

impl<LAYOUT: BufferFormat<INLINE_SMALL, false>, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const RETAIN_INDICES: bool>
From<Vec<u8>> for BufferMutGeneric<LAYOUT, GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, RETAIN_INDICES> {
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
