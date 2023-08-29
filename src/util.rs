use std::alloc::{alloc, alloc_zeroed, Layout};
use std::mem::align_of;
use std::sync::atomic::AtomicUsize;

pub(crate) fn alloc_zeroed_buffer(len: usize) -> *mut u8 {
    // we align the buffer to align_of(usize) bytes for the reference counter to be stored in aligned memory
    let alloc = unsafe { alloc_zeroed(Layout::from_size_align(len, align_of::<AtomicUsize>()).unwrap()) };
    if alloc.is_null() {
        panic!("allocation failure");
    }
    alloc
}

pub(crate) unsafe fn alloc_uninit_buffer(len: usize) -> *mut u8 {
    // we align the buffer to align_of(usize) bytes for the reference counter to be stored in aligned memory
    let alloc = unsafe { alloc(Layout::from_size_align(len, align_of::<AtomicUsize>()).unwrap()) };
    if alloc.is_null() {
        panic!("allocation failure");
    }
    alloc
}

#[inline]
pub(crate) fn find_sufficient_cap<const GROWTH_FACTOR: usize>(curr: usize, req: usize) -> usize {
    let mut curr = curr;
    loop {
        if curr >= req {
            return curr;
        }
        curr *= GROWTH_FACTOR;
    }
}

#[inline]
pub(crate) fn align_to<const ALIGNMENT: usize>(val: usize) -> usize {
    let additional = val % ALIGNMENT;
    let diff = ALIGNMENT - additional;
    val + diff
}