use std::{alloc, ptr};
use std::alloc::{alloc, alloc_zeroed, Layout};

pub(crate) fn alloc_zeroed_buffer(len: usize) -> *mut u8 {
    let alloc = unsafe { alloc_zeroed(Layout::array::<u8>(len).unwrap()) };
    if alloc.is_null() {
        panic!("allocation failure");
    }
    alloc
}

pub(crate) unsafe fn alloc_uninit_buffer(len: usize) -> *mut u8 {
    let alloc = unsafe { alloc(Layout::array::<u8>(len).unwrap()) };
    if alloc.is_null() {
        panic!("allocation failure");
    }
    alloc
}

#[inline]
pub(crate) unsafe fn dealloc(ptr: *mut u8, len: usize) {
    unsafe { alloc::dealloc(ptr, Layout::from_size_align_unchecked(len, 1)); }
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
fn align_to<const ALIGNMENT: usize>(val: usize) -> usize {
    let additional = val % ALIGNMENT;
    let diff = ALIGNMENT - additional;
    val + diff
}

#[inline]
pub(crate) fn align_unaligned_len_to<const ALIGNMENT: usize>(ptr: *mut u8, len: usize) -> usize {
    let raw = ptr as usize;
    let aligned = align_to::<ALIGNMENT>(raw);
    let ptr_diff = aligned - raw;
    if ptr_diff > len {
        ptr_diff
    } else {
        ptr_diff + align_to::<ALIGNMENT>(len - ptr_diff)
    }
}

#[inline]
pub(crate) unsafe fn align_unaligned_ptr_to<const ALIGNMENT: usize>(ptr: *mut u8, len: usize) -> *mut u8 {
    unsafe { ptr.add(align_unaligned_len_to::<ALIGNMENT>(ptr, len)) }
}

#[inline]
pub(crate) unsafe fn realloc_buffer(buf: *mut u8, len: usize, new_cap: usize) -> *mut u8 {
    let alloc = unsafe { alloc_uninit_buffer(new_cap) };
    // copy the previous buffer into the newly allocated one
    unsafe { ptr::copy_nonoverlapping(buf, alloc, len); }
    alloc
}

#[inline]
pub(crate) unsafe fn realloc_buffer_and_dealloc(buf: *mut u8, len: usize, old_cap: usize, new_cap: usize) -> *mut u8 {
    let alloc = unsafe { realloc_buffer(buf, len, new_cap) };
    unsafe { dealloc(buf, old_cap); }
    alloc
}