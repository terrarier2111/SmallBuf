use std::mem::size_of;

use crate::util::{build_bit_mask, COMPRESSED_WORD_SIZE, TAIL_SHIFT, TAIL_MASK, translate_cap, WORD_MASK, round_up_pow_2};

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

const CAP_OFFSET_MASK: usize = build_bit_mask(COMPRESSED_WORD_SIZE, CAP_OFFSET_BITS);
const CAP_OFFSET_SHIFT: usize = TAIL_SHIFT;
const CAP_OFFSET_BITS: usize = usize::BITS.trailing_zeros() as usize;
const CAP_MASK: usize = TAIL_MASK;
const CAP_SHIFT: usize = TAIL_SHIFT;

const RDX_LOWER_MASK: usize = build_bit_mask(usize::BITS as usize / 4 * 3, usize::BITS as usize / 4 * 1);
const RDX_LOWER_SHIFT: usize = RDX_LOWER_MASK.leading_zeros() as usize;

#[derive(Clone, Copy)]
pub(crate) struct ReferenceBuffer {
    pub(crate) wrx: usize,
    pub(crate) offset: usize,
    pub(crate) ptr: *mut u8,
}

// representation on 64 bit systems:

// total available bits: 64 * 3 = 192
// writer index: 40
// reader index: 40
// offset: 40
// len: 40
// flags: 2
// 192 - 40 - 40 - 40 - 40 - 2 = 30
// capacity: 24
// capacity shift: 4
// 30 - 24 - 4 = 2

// -> 2 bits remaining!

// 1. word: len[40 bits], cap_offset [4 bits], rdx_upper[16 bits]
// 2. word: wrx[40 bits], rdx_lower[24 bits]
// 3. word: offset[40 bits], capacity[24 bits]

// FIXME: change design to preserve rdx even inside write only buffers!

impl ReferenceBuffer {

    #[inline]
    pub fn new(cap: usize, offset: usize, wrx: usize, ptr: *mut u8) -> Self {
        Self {
            wrx,
            offset: offset | (cap << CAP_SHIFT),
            ptr,
        }
    }

    #[inline]
    fn capacity_raw(&self) -> usize {
        (self.offset & CAP_MASK) >> CAP_SHIFT
    }

    #[inline]
    fn set_capacity_raw(&mut self, cap: usize) -> usize {
        let (cap, cap_offset) = translate_cap(cap);
        self.offset = (self.offset & WORD_MASK) | (cap << TAIL_SHIFT);
        cap_offset
    }

    #[inline]
    fn wrx(&self) -> usize {
        self.wrx & WORD_MASK
    }

    #[inline]
    fn set_wrx(&mut self, wrx: usize) {
        self.wrx = wrx | (self.wrx & TAIL_MASK);
    }

    #[inline]
    fn rdx_lower(&self) -> usize {
        (self.wrx & TAIL_MASK) >> TAIL_SHIFT
    }

    #[inline]
    fn set_rdx_lower(&mut self, rdx: usize) {
        self.wrx |= rdx << RDX_LOWER_SHIFT;
    }

    #[inline]
    fn offset(&self) -> usize {
        self.offset & !CAP_MASK
    }

    #[inline]
    fn set_offset(&mut self, offset: usize) {
        self.offset &= CAP_MASK;
        self.offset |= offset;
    }

}