use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::mem::{align_of, size_of};
use std::ops::Add;
use std::ptr;
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::AtomicUsize;
use crate::{GenericBuffer, WritableBuffer};
use crate::util::{alloc_uninit_buffer, alloc_zeroed_buffer, find_sufficient_cap};

#[repr(C)]
pub struct BufferMut {
    pub(crate) len: usize,
    pub(crate) cap: usize,
    pub(crate) ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_FLAG: usize = 1 << (usize::BITS - 1);
pub(crate) const INLINE_SIZE: usize = size_of::<BufferMut>() - size_of::<usize>();
const INITIAL_CAP: usize = INLINE_SIZE * GROWTH_FACTOR;
const GROWTH_FACTOR: usize = 2;
const ADDITIONAL_BUFFER_CAP: usize = size_of::<AtomicUsize>();

unsafe impl Send for BufferMut {}
unsafe impl Sync for BufferMut {}

impl Clone for BufferMut {
    #[inline]
    fn clone(&self) -> Self {
        if self.len & INLINE_FLAG != 0 {
            return Self {
                len: self.len,
                cap: self.cap,
                ptr: self.ptr,
            };
        }

        let alloc = unsafe { alloc_uninit_buffer(self.cap) };
        unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len()); }
        Self {
            len: self.len,
            cap: self.cap,
            ptr: alloc,
        }
    }
}

impl AsRef<[u8]> for BufferMut {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.len & INLINE_FLAG != 0 {
            unsafe { (self as *const BufferMut).cast::<u8>().add(size_of::<usize>()) }
        } else {
            self.ptr
        };
        unsafe { &*slice_from_raw_parts(ptr, self.len()) }
    }
}

impl GenericBuffer for BufferMut {
    #[inline]
    fn new() -> Self {
        Self {
            len: 0 | INLINE_FLAG,
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
    }

    fn shrink(&mut self) {
        todo!()
    }
}

impl BufferMut {

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        if self.len & INLINE_FLAG != 0 {
            if unsafe { self.len() } + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer(buffer: *mut BufferMut, req: usize) -> *mut u8 {
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
                return outline_buffer(self as *mut BufferMut, req);
            }
            return unsafe { (self as *mut BufferMut).cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req {
            #[inline(never)]
            #[cold]
            fn resize_alloc(buffer: *mut BufferMut, req: usize) {
                let new_cap = find_sufficient_cap::<GROWTH_FACTOR>(unsafe { (&*buffer).cap }, req);
                unsafe { (&mut *buffer).cap = new_cap; }
                let old_alloc = unsafe { (&*buffer).ptr };
                unsafe { (&mut *buffer).ptr = unsafe { alloc_uninit_buffer((&*buffer).cap) }; }
                unsafe { ptr::copy_nonoverlapping(old_alloc, (&*buffer).ptr, (&*buffer).len); }
            }
            resize_alloc(self as *mut BufferMut, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

}

impl WritableBuffer for BufferMut {

    fn with_capacity(cap: usize) -> Self {
        if cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_FLAG,
                // the following two values are now treated as the buffer
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            // we allocate an additional size_of(usize) bytes for the reference counter to be stored
            let alloc = unsafe { alloc_uninit_buffer(cap + ADDITIONAL_BUFFER_CAP) };
            Self {
                len: 0,
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
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            // we allocate an additional size_of(usize) bytes for the reference counter to be stored
            let alloc = alloc_zeroed_buffer(len + ADDITIONAL_BUFFER_CAP);
            Self {
                len,
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

impl Drop for BufferMut {
    #[inline]
    fn drop(&mut self) {
        if self.len & INLINE_FLAG == 0 {
            unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.cap, align_of::<AtomicUsize>())); }
        }
    }
}
