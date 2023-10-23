use std::mem::size_of;

use crate::util::round_up_pow_2;

/// This is the size of the `inlined` field in `BufferUnion`
pub(crate) const INLINE_SIZE: usize = size_of::<[usize; 3]>();
pub(crate) const INLINE_SIZE_BITS: usize = round_up_pow_2(INLINE_SIZE).trailing_zeros() as usize;

#[derive(Clone)]
pub(crate) struct BaseBuffer {
    pub(crate) len: usize,
    pub(crate) buffer: BufferUnion,
}

#[derive(Copy, Clone)]
pub(crate) union BufferUnion {
    pub(crate) inlined: [usize; 3],
    pub(crate) reference: ReferenceBuffer,
}

#[derive(Clone, Copy)]
pub(crate) struct ReferenceBuffer {
    pub(crate) wrx: usize,
    pub(crate) offset: usize,
    pub(crate) ptr: *mut u8,
}