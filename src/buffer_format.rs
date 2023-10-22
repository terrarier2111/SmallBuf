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

impl BaseBuffer {

    #[inline]
    pub fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: usize) -> Self {
        let (cap, cap_offset) = translate_cap(cap);
        let len = len | flags | (cap_offset << CAP_OFFSET_SHIFT);
        Self {
            len,
            buffer: BufferUnion { reference: ReferenceBuffer::new(cap, offset, wrx, ptr), },
        }
    }

    #[inline]
    pub fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self {
        Self {
            len: len | (offset << INLINE_OFFSET_SHIFT),
            buffer: BufferUnion { inlined: [0; 3], },
        }
    }

    #[inline]
    pub fn len_reference(&self) -> usize {
        self.len & WORD_MASK
    }

    #[inline]
    pub fn len_inlined(&self) -> usize {
        self.len & INLINE_LEN_MASK
    }

    #[inline]
    pub fn set_len_reference(&mut self, len: usize) {
        self.len = (self.len & TAIL_MASK) | len;
    }

    #[inline]
    pub fn set_len_inlined(&mut self, len: usize) {
        self.len = (self.len & !INLINE_LEN_MASK) | len;
    }

    #[inline]
    pub fn offset_reference(&self) -> usize {
        self.buffer.reference.offset & WORD_MASK
    }

    #[inline]
    pub fn offset_inlined(&self) -> usize {
        (self.len & INLINE_OFFSET_MASK) >> INLINE_OFFSET_SHIFT
    }

    #[inline]
    pub fn set_offset_reference(&mut self, offset: usize) {
        self.buffer.reference.offset = (self.buffer.reference.offset & TAIL_MASK) | offset;
    }

    #[inline]
    pub fn set_offset_inlined(&mut self, offset: usize) {
        self.len = (self.len & !INLINE_OFFSET_MASK) | (offset << INLINE_OFFSET_SHIFT);
    }

    #[inline]
    pub fn wrx_reference(&self) -> usize {
        self.buffer.reference.wrx & WORD_MASK
    }

    #[inline]
    pub fn wrx_inlined(&self) -> usize {
        (self.len & INLINE_WRX_MASK) >> INLINE_WRX_SHIFT
    }

    #[inline]
    pub fn set_wrx_reference(&mut self, wrx: usize) {
        self.buffer.reference.wrx = (self.buffer.reference.wrx & TAIL_MASK) | wrx;
    }

    #[inline]
    pub fn set_wrx_inlined(&mut self, wrx: usize) {
        self.len = (self.len & !INLINE_WRX_MASK) | (wrx << INLINE_WRX_SHIFT);
    }

    #[inline]
    pub fn rdx_reference(&self) -> usize {
        ((self.buffer.reference.wrx & RDX_LOWER_MASK) >> RDX_LOWER_SHIFT) | ((self.len & RDX_UPPER_MASK) >> (RDX_UPPER_SHIFT - RDX_LOWER_SHIFT))
    }

    #[inline]
    pub fn rdx_inlined(&self) -> usize {
        (self.len & INLINE_RDX_MASK) >> INLINE_RDX_SHIFT
    }

    #[inline]
    pub fn set_rdx_reference(&mut self, rdx: usize) {
        self.buffer.reference.wrx = (self.buffer.reference.wrx & WORD_MASK) | (rdx << RDX_LOWER_SHIFT);
        self.len = (self.len & !RDX_UPPER_MASK) | ((rdx << (RDX_UPPER_SHIFT - RDX_LOWER_SHIFT)) & RDX_UPPER_MASK);
    }

}

union BufferUnion {
    inlined: [usize; 3],
    reference: ReferenceBuffer,
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
    fn new(cap: usize, offset: usize, wrx: usize, ptr: *mut u8) -> Self {
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