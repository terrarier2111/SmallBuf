use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::Deref;
use std::{mem, ptr};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::AtomicUsize;
use crate::{GenericBuffer, WritableBuffer};
use crate::util::{align_unaligned_len_to, alloc_uninit_buffer, alloc_zeroed_buffer, dealloc, find_sufficient_cap, realloc_buffer, realloc_buffer_and_dealloc};

pub type BufferMut = BufferMutGeneric;

#[repr(C)]
pub struct BufferMutGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = { GROWTH_FACTOR * INLINE_SIZE }, const INLINE_SMALL: bool = true, const FAST_CONVERSION: bool = true> {
    pub(crate) len: usize,
    pub(crate) cap: usize,
    pub(crate) ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_FLAG: usize = 1 << (usize::BITS - 1);
pub(crate) const INLINE_SIZE: usize = size_of::<BufferMutGeneric<0, 0, false, false>>() - size_of::<usize>();
const ADDITIONAL_BUFFER_CAP: usize = size_of::<AtomicUsize>();

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Send for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Sync for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Clone for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    #[inline]
    fn clone(&self) -> Self {
        if self.is_inlined() {
            return Self {
                len: self.len,
                cap: self.cap,
                ptr: self.ptr,
            };
        }

        let alloc = unsafe { realloc_buffer(self.ptr, self.len, self.cap) };
        Self {
            len: self.len,
            cap: self.cap,
            ptr: alloc,
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
AsRef<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if self.is_inlined() {
            unsafe { (self as *const BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { FAST_CONVERSION }>).cast::<u8>().add(size_of::<usize>()) }
        } else {
            self.ptr
        };
        unsafe { &*slice_from_raw_parts(ptr, self.len()) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Deref for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Borrow<[u8]> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Into<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    #[inline]
    fn into(self) -> Vec<u8> {
        let ret = unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) };
        mem::forget(self);
        ret
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
From<Vec<u8>> for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
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
            let ret_ptr = &mut ret as *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION>;
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
GenericBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
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
        if self.is_inlined() {
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
        if self.is_inlined() {
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
        let alloc = unsafe { realloc_buffer(self.ptr, self.len, target_cap) };
        self.ptr = alloc;
        self.cap = target_cap;
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {

    #[inline]
    pub(crate) fn is_inlined(&self) -> bool {
        INLINE_SMALL && self.len & INLINE_FLAG != 0
    }

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        let ptr = self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { FAST_CONVERSION }>;
        if self.is_inlined() {
            if self.len() + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION>, req: usize) -> *mut u8 {
                    // remove the inline flag
                    unsafe { (&mut *buffer).len &= !INLINE_FLAG; }
                    let cap = find_sufficient_cap::<GROWTH_FACTOR>(INITIAL_CAP, if FAST_CONVERSION {
                        // we allocate an additional size_of(usize) * 2 - 1 bytes for the reference counter to be stored
                        req + ADDITIONAL_BUFFER_CAP
                    } else {
                        req
                    });
                    let len = unsafe { (&*buffer).len };
                    let alloc = unsafe { realloc_buffer(buffer.cast::<u8>().add(size_of::<usize>()), len, cap) };
                    unsafe { (&mut *buffer).cap = cap };
                    unsafe { (&mut *buffer).ptr = alloc };
                    unsafe { alloc.add(len) }
                }
                // handle outlining buffer
                return outline_buffer(ptr, req);
            }
            return unsafe { ptr.cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // handle buffer reallocation
        if self.cap < self.len + req {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION>, req: usize) {
                let old_cap = unsafe { (&*buffer).cap };
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(old_cap, if FAST_CONVERSION {
                    // we allocate an additional size_of(usize) * 2 - 1 bytes for the reference counter to be stored
                    req + ADDITIONAL_BUFFER_CAP
                } else {
                    req
                });
                let alloc = unsafe { realloc_buffer_and_dealloc((&*buffer).ptr, (&*buffer).len, old_cap, new_cap) };
                unsafe { (&mut *buffer).ptr = alloc; }
                unsafe { (&mut *buffer).cap = new_cap; }
            }
            resize_alloc(self as *mut BufferMutGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { FAST_CONVERSION }>, req);
        }
        unsafe { self.ptr.add(self.len) }
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
WritableBuffer for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {

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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const FAST_CONVERSION: bool>
Drop for BufferMutGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, FAST_CONVERSION> {
    #[inline]
    fn drop(&mut self) {
        if self.is_inlined() {
            // we don't need to do anything for inlined buffers
            return;
        }
        if !INLINE_SMALL && self.ptr.is_null() {
            // we don't do anything if there is no allocation
            return;
        }
        unsafe { dealloc(self.ptr, self.cap); }
    }
}
