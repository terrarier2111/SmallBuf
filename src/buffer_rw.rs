use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::mem::{align_of, size_of};
use std::ops::Add;
use std::ptr;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::AtomicUsize;
use crate::util::{alloc_uninit_buffer, alloc_zeroed_buffer, find_sufficient_cap};
use crate::{GenericBuffer, ReadableBuffer, RWBuffer, WritableBuffer};

#[repr(C)]
pub struct BufferRW {
    len: usize,
    rdx: usize,
    cap: usize,
    ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_FLAG: usize = 1 << (usize::BITS - 1);
const INLINE_SIZE: usize = size_of::<BufferRW>() - size_of::<usize>();
const INITIAL_CAP: usize = INLINE_SIZE * GROWTH_FACTOR;
const GROWTH_FACTOR: usize = 2;
const ADDITIONAL_BUFFER_CAP: usize = size_of::<AtomicUsize>();

unsafe impl Send for BufferRW {}
unsafe impl Sync for BufferRW {}

impl BufferRW {

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        if self.len & INLINE_FLAG != 0 {
            if unsafe { self.len() } + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer(buffer: *mut BufferRW, req: usize) -> *mut u8 {
                    // remove the inline flag
                    unsafe { (&mut *buffer).len &= !INLINE_FLAG; }
                    // we allocate an additional size_of(usize) bytes for the reference counter to be stored
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, req + ADDITIONAL_BUFFER_CAP);
                    let alloc = alloc_zeroed_buffer(cap);
                    let len = unsafe { (&*buffer).len };
                    // copy the previous buffer into the newly allocated one
                    unsafe { ptr::copy_nonoverlapping(unsafe { buffer.cast::<u8>().add(size_of::<usize>()) }, alloc, len); }
                    unsafe { (&mut *buffer).cap = cap };
                    unsafe { (&mut *buffer).ptr = alloc };
                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(self as *mut BufferRW, req);
            }
            return unsafe { (self as *mut BufferRW).cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req {
            #[inline(never)]
            #[cold]
            fn resize_alloc(buffer: *mut BufferRW, req: usize) {
                let new_cap = find_sufficient_cap::<GROWTH_FACTOR>(unsafe { (&*buffer).cap }, req);
                unsafe { (&mut *buffer).cap = new_cap; }
                let old_alloc = unsafe { (&*buffer).ptr };
                unsafe { (&mut *buffer).ptr = unsafe { alloc_uninit_buffer((&*buffer).cap) }; }
                unsafe { ptr::copy_nonoverlapping(old_alloc, (&*buffer).ptr, (&*buffer).len); }
            }
            resize_alloc(self as *mut BufferRW, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

    #[inline]
    fn ensure_readable(&self, bytes: usize) -> *const u8 {
        let remaining = self.remaining();
        if remaining < bytes {
            panic!("not enough bytes in buffer, expected {} readable bytes but only {} bytes are left", bytes, remaining);
        }
        if self.len & INLINE_FLAG != 0 {
            unsafe { (self as *const BufferRW).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        }
    }

}

impl GenericBuffer for BufferRW {
    #[inline]
    fn new() -> Self {
        Self {
            len: 0 | INLINE_FLAG,
            rdx: 0,
            cap: 0,
            ptr: null_mut(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len & !INLINE_FLAG
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if self.len & INLINE_FLAG != 0 {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn clear(&mut self) {
        self.len = 0 | (self.len & INLINE_FLAG);
        self.rdx = 0;
    }

    fn shrink(&mut self) {
        todo!()
    }
}

impl Clone for BufferRW {
    #[inline]
    fn clone(&self) -> Self {
        if self.len & INLINE_FLAG != 0 {
            return Self {
                len: self.len,
                rdx: self.rdx,
                cap: self.cap,
                ptr: self.ptr,
            };
        }

        let alloc = unsafe { alloc_uninit_buffer(self.cap) };
        unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len()); }
        Self {
            len: self.len,
            rdx: self.rdx,
            cap: self.cap,
            ptr: alloc,
        }
    }
}

impl AsRef<[u8]> for BufferRW {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.len & INLINE_FLAG != 0 {
            unsafe { (self as *const BufferRW).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

impl WritableBuffer for BufferRW {

    fn with_capacity(cap: usize) -> Self {
        if cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            // we allocate an additional size_of(usize) bytes for the reference counter to be stored
            let alloc = unsafe { alloc_uninit_buffer(cap + ADDITIONAL_BUFFER_CAP) };
            Self {
                len: 0,
                rdx: 0,
                cap: cap + ADDITIONAL_BUFFER_CAP,
                ptr: alloc,
            }
        }
    }

    fn zeroed(len: usize) -> Self {
        if len <= INLINE_SIZE {
            Self {
                len: len | INLINE_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            // we allocate an additional size_of(usize) bytes for the reference counter to be stored
            let alloc = alloc_zeroed_buffer(len + ADDITIONAL_BUFFER_CAP);
            Self {
                len,
                rdx: 0,
                cap: len + ADDITIONAL_BUFFER_CAP,
                ptr: alloc,
            }
        }
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

impl ReadableBuffer for BufferRW {
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

impl RWBuffer for BufferRW {}

impl Drop for BufferRW {
    #[inline]
    fn drop(&mut self) {
        if self.len & INLINE_FLAG == 0 {
            unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.cap, align_of::<AtomicUsize>())); }
        }
    }
}