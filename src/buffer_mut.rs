use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::{Add, Deref};
use std::{mem, ptr};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::AtomicUsize;
use crate::{GenericBuffer, WritableBuffer};
use crate::util::{align_unaligned_len_to, alloc_uninit_buffer, alloc_zeroed_buffer, dealloc, find_sufficient_cap};

pub type BufferMut = BufferMutGeneric;

#[repr(C)]
pub struct BufferMutGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = { GROWTH_FACTOR * INLINE_SIZE }, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true, const FAST_CONVERSION: bool = true> {
    pub(crate) len: usize,
    pub(crate) cap: usize,
    pub(crate) ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_FLAG: usize = 1 << (usize::BITS - 1);
pub(crate) const INLINE_SIZE: usize = size_of::<BufferMutGeneric<0, 0, false, false, false>>() - size_of::<usize>();
const ADDITIONAL_BUFFER_CAP: usize = size_of::<AtomicUsize>();

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Send for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Sync for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Clone for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn clone(&self) -> Self {
        if INLINE_SMALL && self.len & INLINE_FLAG != 0 {
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
AsRef<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if INLINE_SMALL && self.len & INLINE_FLAG != 0 {
            unsafe { (self as *const BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>).cast::<u8>().add(size_of::<usize>()) }
        } else {
            self.ptr
        };
        unsafe { &*slice_from_raw_parts(ptr, self.len()) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Deref for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Borrow<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Into<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn into(self) -> Vec<u8> {
        unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            let mut ret = Self {
                len,
                cap: 0,
                ptr: null_mut(),
            };
            let ret_ptr = &mut ret as *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>;
            unsafe { ptr::copy_nonoverlapping(ptr, ret_ptr.cast::<u8>().add(size_of::<usize>()), len); }
            return ret;
        }
        mem::forget(value);
        // reuse existing buffer
        Self {
            len,
            cap,
            ptr,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
GenericBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_FLAG
            } else {
                0
            },
            cap: 0,
            ptr: null_mut(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        if INLINE_SMALL {
            self.len & !INLINE_FLAG
        } else {
            self.len
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if INLINE_SMALL && self.len & INLINE_FLAG != 0 {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn clear(&mut self) {
        if INLINE_SMALL {
            self.len = 0 | (self.len & INLINE_FLAG);
        } else {
            self.len = 0;
        }
    }

    fn shrink(&mut self) {
        if INLINE_SMALL && self.len & INLINE_FLAG != 0 {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        let target_cap = if FAST_CONVERSION {
            align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(self.ptr, self.len) + size_of::<AtomicUsize>()
        } else {
            self.len
        };
        if self.cap >= target_cap {
            // we have nothing to do as our capacity is already as small as possible
            return;
        }
        let alloc = unsafe { alloc_uninit_buffer(target_cap) };
        unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len); }
        self.ptr = alloc;
        self.cap = target_cap;
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        if INLINE_SMALL && self.len & INLINE_FLAG != 0 {
            if self.len() + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>, req: usize) -> *mut u8 {
                    // remove the inline flag
                    unsafe { (&mut *buffer).len &= !INLINE_FLAG; }
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, if FAST_CONVERSION {
                        // we allocate an additional size_of(usize) * 2 - 1 bytes for the reference counter to be stored
                        req + ADDITIONAL_BUFFER_CAP
                    } else {
                        req
                    });
                    let alloc = alloc_zeroed_buffer(cap);
                    let len = unsafe { (&*buffer).len };
                    // copy the previous buffer into the newly allocated one
                    unsafe { ptr::copy_nonoverlapping(unsafe { buffer.cast::<u8>().add(size_of::<usize>()) }, alloc, len); }
                    unsafe { (&mut *buffer).cap = cap };
                    unsafe { (&mut *buffer).ptr = alloc };
                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>, req);
            }
            return unsafe { (self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>).cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>, req: usize) {
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(unsafe { (&*buffer).cap }, req);
                unsafe { (&mut *buffer).cap = new_cap; }
                let old_alloc = unsafe { (&*buffer).ptr };
                unsafe { (&mut *buffer).ptr = unsafe { alloc_uninit_buffer((&*buffer).cap) }; }
                unsafe { ptr::copy_nonoverlapping(old_alloc, (&*buffer).ptr, (&*buffer).len); }
            }
            resize_alloc(self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
WritableBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_FLAG,
                // the following two values are now treated as the buffer
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            let cap = if FAST_CONVERSION {
                // we allocate an additional size_of(usize) bytes for the reference counter to be stored
                // and all possible required alignment storage
                cap + ADDITIONAL_BUFFER_CAP * 2 - 1
            } else {
                cap
            };
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            Self {
                len: 0,
                cap,
                ptr: alloc,
            }
        }
    }

    fn zeroed(len: usize) -> Self {
        if INLINE_SMALL && len <= INLINE_SIZE {
            Self {
                len: len | INLINE_FLAG,
                // the following two values are now treated as the buffer
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            let cap = if FAST_CONVERSION {
                // we allocate an additional size_of(usize) bytes for the reference counter to be stored
                // and all possible required alignment storage
                len + ADDITIONAL_BUFFER_CAP * 2 - 1
            } else {
                len
            };
            let alloc = alloc_zeroed_buffer(cap);
            Self {
                len,
                cap,
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Drop for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn drop(&mut self) {
        if !INLINE_SMALL || self.len & INLINE_FLAG == 0 {
            if INLINE_SMALL || !self.ptr.is_null() {
                unsafe { dealloc(self.ptr, self.cap); }
            }
        }
    }
}
