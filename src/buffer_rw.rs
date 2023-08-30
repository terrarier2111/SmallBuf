use std::borrow::Borrow;
use std::mem::{align_of, size_of};
use std::ops::{Add, Deref};
use std::{mem, ptr};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::util::{align_unaligned_len_to, align_unaligned_ptr_to, alloc_uninit_buffer, alloc_zeroed_buffer, dealloc, find_sufficient_cap};
use crate::{GenericBuffer, ReadableBuffer, RWBuffer, WritableBuffer};
use crate::buffer::{Buffer, BufferGeneric};
use crate::buffer_mut::{BufferMut, BufferMutGeneric};

pub type BufferRW = BufferRWGeneric;

#[repr(C)]
pub struct BufferRWGeneric<const GROWTH_FACTOR: usize = 2, const INITIAL_CAP: usize = { GROWTH_FACTOR * INLINE_SIZE }, const INLINE_SMALL: bool = true, const STATIC_STORAGE: bool = true, const FAST_CONVERSION: bool = true> {
    len: usize,
    rdx: usize,
    cap: usize,
    ptr: *mut u8,
}

/// the MSB will never be used as allocations are capped at isize::MAX
const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
/// the MSB will never be used as allocations are capped at isize::MAX
const STATIC_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
const INLINE_SIZE: usize = size_of::<BufferRWGeneric::<0, 0, false, false, false>>() - size_of::<usize>();
const ADDITIONAL_BUFFER_CAP: usize = size_of::<AtomicUsize>() * 2 - 1;

unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Send for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}
unsafe impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Sync for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

    #[inline]
    fn ensure_large_enough(&mut self, req: usize) -> *mut u8 {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            if self.len() + req > INLINE_SIZE {
                #[cold]
                #[inline(never)]
                fn outline_buffer<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>, req: usize) -> *mut u8 {
                    // remove the inline flag
                    unsafe { (&mut *buffer).len &= !INLINE_BUFFER_FLAG; }
                    let cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(INITIAL_CAP, if FAST_CONVERSION {
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
                return outline_buffer::<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>(self as *mut BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>, req);
            }
            return unsafe { (self as *mut BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>).cast::<u8>().add(usize::BITS as usize / 8 + self.len()) };
        }
        // move the static buffer into a dynamic heap buffer
        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
            let cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(INITIAL_CAP, if FAST_CONVERSION {
                // we allocate an additional size_of(usize) * 2 - 1 bytes for the reference counter to be stored
                req + ADDITIONAL_BUFFER_CAP
            } else {
                req
            });
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len); }
            self.ptr = alloc;
            self.cap = cap;
            // mark the buffer as non static
            self.rdx &= !STATIC_BUFFER_FLAG;
        }
        // handle buffer reallocation
        if self.cap < self.len + req {
            #[inline(never)]
            #[cold]
            fn resize_alloc<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>(buffer: *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>, req: usize) {
                let old_cap = unsafe { (&*buffer).cap };
                let new_cap = find_sufficient_cap::<{ GROWTH_FACTOR }>(old_cap, if FAST_CONVERSION {
                    // we allocate an additional size_of(usize) * 2 - 1 bytes for the reference counter to be stored
                    req + ADDITIONAL_BUFFER_CAP
                } else {
                    req
                });
                unsafe { (&mut *buffer).cap = new_cap; }
                let old_alloc = unsafe { (&*buffer).ptr };
                unsafe { (&mut *buffer).ptr = unsafe { alloc_uninit_buffer((&*buffer).cap) }; }
                unsafe { ptr::copy_nonoverlapping(old_alloc, (&*buffer).ptr, (&*buffer).len); }
                unsafe { dealloc(old_alloc, old_cap); }
            }
            resize_alloc(self as *mut BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>, req);
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
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const BufferRWGeneric<{ GROWTH_FACTOR }, { INITIAL_CAP }, { INLINE_SMALL }, { STATIC_STORAGE }, { FAST_CONVERSION }>).cast::<u8>().add(size_of::<usize>() * 2 + rdx) }
        } else {
            unsafe { self.ptr.add(rdx) }
        }
    }

}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Deref for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Borrow<[u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Into<Vec<u8>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn into(self) -> Vec<u8> {
        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
            let cap = if FAST_CONVERSION {
                self.len + ADDITIONAL_BUFFER_CAP
            } else {
                self.len
            };
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len); }
            return unsafe { Vec::from_raw_parts(self.ptr, self.len, cap) };
        }
        unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<Vec<u8>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn from(mut value: Vec<u8>) -> Self {
        let ptr = value.as_mut_ptr();
        let cap = value.capacity();
        let len = value.len();
        // handle small buffers
        if INLINE_SMALL && len <= INLINE_SIZE {
            let mut ret = Self {
                len,
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            };
            let ret_ptr = &mut ret as *mut BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>;
            unsafe { ptr::copy_nonoverlapping(ptr, ret_ptr.cast::<u8>().add(size_of::<usize>() * 2), len); }
            return ret;
        }
        mem::forget(value);
        // reuse existing buffer
        Self {
            len,
            rdx: 0,
            cap,
            ptr,
        }
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const FAST_CONVERSION_OTHER: bool, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn from(value: BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>) -> Self {
        if INLINE_SMALL && value.len & INLINE_BUFFER_FLAG != 0 {
            return Self {
                len: value.len,
                rdx: value.rdx,
                cap: value.cap,
                ptr: value.ptr,
            };
        }
        if STATIC_STORAGE && value.rdx & STATIC_BUFFER_FLAG != 0 {
            return Self {
                len: value.len,
                rdx: value.rdx,
                cap: value.cap,
                ptr: value.ptr,
            };
        }
        let refs = unsafe { &*align_unaligned_ptr_to::<{ align_of::<AtomicUsize>() }>(value.ptr, value.len).cast::<AtomicUsize>() }.load(Ordering::Acquire);
        if refs == 1 {
            let ret = Self {
                len: value.len,
                rdx: value.rdx,
                cap: value.cap,
                ptr: value.ptr,
            };
            mem::forget(value);
            return ret;
        }
        // TODO: should we try to shrink?
        let alloc = unsafe { alloc_uninit_buffer(value.cap) };
        unsafe { ptr::copy_nonoverlapping(value.ptr, alloc, value.len); }
        Self {
            len: value.len,
            rdx: value.rdx,
            cap: value.cap,
            ptr: alloc,
        }
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const FAST_CONVERSION_OTHER: bool, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Into<BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn into(self) -> BufferGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER> {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            return BufferGeneric {
                len: self.len,
                rdx: self.rdx,
                cap: self.cap,
                ptr: self.ptr,
            };
        }
        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
            return BufferGeneric {
                len: self.len,
                rdx: self.rdx,
                cap: self.cap,
                ptr: self.ptr,
            };
        }
        let available = self.cap - self.len;
        let aligned_len = align_unaligned_len_to::<{ align_of::<AtomicUsize>() }>(self.ptr, self.len) + size_of::<AtomicUsize>();
        if available < aligned_len {
            let alloc = unsafe { alloc_uninit_buffer(self.len + ADDITIONAL_BUFFER_CAP) };
            unsafe { ptr::copy_nonoverlapping(self.ptr, alloc, self.len); }
            return BufferGeneric {
                len: self.len,
                rdx: self.rdx,
                cap: self.len + ADDITIONAL_BUFFER_CAP,
                ptr: alloc,
            };
        }
        let ret = BufferGeneric {
            len: self.len,
            rdx: self.rdx,
            cap: self.cap,
            ptr: self.ptr,
        };
        mem::forget(self);
        ret
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const FAST_CONVERSION_OTHER: bool, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
From<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn from(value: BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>) -> Self {
        let ret = Self {
            len: value.len,
            rdx: 0,
            cap: value.cap,
            ptr: value.ptr,
        };
        mem::forget(value);
        ret
    }
}

impl<const GROWTH_FACTOR_OTHER: usize, const INITIAL_CAP_OTHER: usize, const FAST_CONVERSION_OTHER: bool, const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Into<BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER>> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn into(self) -> BufferMutGeneric<GROWTH_FACTOR_OTHER, INITIAL_CAP_OTHER, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION_OTHER> {
        let ret = BufferMutGeneric {
            len: self.len,
            cap: self.cap,
            ptr: self.ptr,
        };
        mem::forget(self);
        ret
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
GenericBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn new() -> Self {
        Self {
            len: if INLINE_SMALL {
                0 | INLINE_BUFFER_FLAG
            } else {
                0
            },
            rdx: 0,
            cap: 0,
            ptr: null_mut(),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        if INLINE_SMALL {
            self.len & !INLINE_BUFFER_FLAG
        } else {
            self.len
        }
    }

    #[inline]
    fn capacity(&self) -> usize {
        // for inlined buffers we always have INLINE_SIZE space
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            return INLINE_SIZE;
        }
        self.cap
    }

    #[inline]
    fn clear(&mut self) {
        if INLINE_SMALL {
            self.len = 0 | (self.len & INLINE_BUFFER_FLAG);
        } else {
            self.len = 0;
        }
        self.rdx = 0;
    }

    fn shrink(&mut self) {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            // we have nothing to do as the buffer is stored in line
            return;
        }
        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
            // we have nothing to do as the buffer is static
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
Clone for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn clone(&self) -> Self {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            return Self {
                len: self.len,
                rdx: self.rdx,
                cap: self.cap,
                ptr: self.ptr,
            };
        }

        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
AsRef<[u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        let ptr = if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            unsafe { (self as *const BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION>).cast::<u8>().add(size_of::<usize>() * 2 + self.rdx) }
        } else {
            unsafe { self.ptr.add(self.rdx) }
        };
        unsafe { &*slice_from_raw_parts(ptr, self.remaining()) }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
WritableBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {

    fn with_capacity(cap: usize) -> Self {
        if INLINE_SMALL && cap <= INLINE_SIZE {
            Self {
                len: 0 | INLINE_BUFFER_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            let cap = if FAST_CONVERSION {
                // we allocate an additional size_of(usize) bytes for the reference counter to be stored
                // and all possible required alignment storage
                cap + ADDITIONAL_BUFFER_CAP
            } else {
                cap
            };
            let alloc = unsafe { alloc_uninit_buffer(cap) };
            Self {
                len: 0,
                rdx: 0,
                cap,
                ptr: alloc,
            }
        }
    }

    fn zeroed(len: usize) -> Self {
        if INLINE_SMALL && len <= INLINE_SIZE {
            Self {
                len: len | INLINE_BUFFER_FLAG,
                // the following two values are now treated as the buffer
                rdx: 0,
                cap: 0,
                ptr: null_mut(),
            }
        } else {
            let cap = if FAST_CONVERSION {
                // we allocate an additional size_of(usize) bytes for the reference counter to be stored
                // and all possible required alignment storage
                len + ADDITIONAL_BUFFER_CAP
            } else {
                len
            };
            let alloc = alloc_zeroed_buffer(cap);
            Self {
                len,
                rdx: 0,
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
From<&'static [u8]> for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self {
            len: value.len(),
            rdx: 0 | STATIC_BUFFER_FLAG,
            cap: 0,
            ptr: value.as_ptr().cast_mut(),
        }
    }
}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
ReadableBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
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

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
RWBuffer for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {}

impl<const GROWTH_FACTOR: usize, const INITIAL_CAP: usize, const INLINE_SMALL: bool, const STATIC_STORAGE: bool, const FAST_CONVERSION: bool>
Drop for BufferRWGeneric<GROWTH_FACTOR, INITIAL_CAP, INLINE_SMALL, STATIC_STORAGE, FAST_CONVERSION> {
    #[inline]
    fn drop(&mut self) {
        if INLINE_SMALL && self.len & INLINE_BUFFER_FLAG != 0 {
            // we don't need to do anything for inlined buffers
            return;
        }
        if STATIC_STORAGE && self.rdx & STATIC_BUFFER_FLAG != 0 {
            // we don't need to do anything for static buffers
            return;
        }
        if !INLINE_SMALL && self.ptr.is_null() {
            // we don't do anything if there is no allocation
            return;
        }
        unsafe { dealloc(self.ptr, self.cap); }
    }
}