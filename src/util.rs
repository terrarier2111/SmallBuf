use std::{alloc, ptr};
use std::alloc::{alloc, alloc_zeroed, Layout};
use std::mem::{align_of, size_of, transmute};

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

/// this offsets dst by OFFSET if src and dst are equal.
/// this implementation is special because it is fully branchless.
///
/// SAFETY: dst has to be greater than src.
///         adding offset to dst has to produce a valid pointer.
#[inline]
pub(crate) unsafe fn offset_if_equal<const OFFSET: usize>(src: *mut u8, dst: *mut u8) -> *mut u8 {
    let diff = dst as usize - src as usize;
    let is_zero = (!diff).overflowing_add(1).1;
    unsafe { dst.add(transmute::<bool, u8>(is_zero) as usize) }
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
pub(crate) unsafe fn align_unaligned_ptr_to<const ALIGNMENT: usize, const REGION_SIZE: usize>(ptr: *mut u8, len: usize) -> *mut u8 {
    let end = unsafe { ptr.add(len) };
    let additional = end as usize % ALIGNMENT;
    let end = end.sub(additional + REGION_SIZE);
    end
}

#[inline]
pub(crate) unsafe fn realloc_buffer_counted(buf: *mut u8, len: usize, new_cap: usize) -> *mut u8 {
    let alloc = unsafe { alloc_uninit_buffer(new_cap) };
    // copy the previous buffer into the newly allocated one
    unsafe { ptr::copy_nonoverlapping(buf, alloc, len); }

    // setup metadata

    let meta_ptr = unsafe { align_unaligned_ptr_to::<{ align_of::<usize>() }, METADATA_SIZE>(alloc, new_cap) };
    assert_eq!(meta_ptr.cast::<usize>() as usize % 8, 0);
    // set ref cnt
    unsafe { *meta_ptr.cast::<usize>() = 1; }
    alloc
}

/// this additional storage is used to store the reference counter and
/// capacity and to align said values properly.
const ADDITIONAL_SIZE: usize = METADATA_SIZE + align_of::<usize>() - 1;
const METADATA_SIZE: usize = size_of::<usize>() * 1;

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

#[inline]
pub(crate) const fn min(left: usize, right: usize) -> usize {
    if left < right {
        left
    } else {
        right
    }
}

static EMPTY_SENTINEL: u8 = 0;

#[inline]
pub(crate) fn empty_sentinel() -> *mut u8 {
    (&EMPTY_SENTINEL as *const u8).cast_mut()
}