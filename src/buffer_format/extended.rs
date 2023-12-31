use crate::{buffer_layout::{BaseBuffer, INLINE_SIZE_BITS, BufferUnion, ReferenceBuffer}, util::{build_bit_mask, round_up_pow_2, greater_zero_ret_one}};

use super::{Flags, BufferFormat};

const INLINE_LEN_MASK: usize = build_bit_mask(0, INLINE_SIZE_BITS);
const INLINE_OFFSET_MASK: usize = build_bit_mask(INLINE_OFFSET_SHIFT, INLINE_SIZE_BITS);
const INLINE_OFFSET_SHIFT: usize = INLINE_SIZE_BITS;
const INLINE_WRX_MASK: usize = build_bit_mask(INLINE_WRX_SHIFT, INLINE_SIZE_BITS);
const INLINE_WRX_SHIFT: usize = INLINE_SIZE_BITS * 2;
const INLINE_RDX_MASK: usize = build_bit_mask(INLINE_RDX_SHIFT, INLINE_SIZE_BITS);
const INLINE_RDX_SHIFT: usize = INLINE_SIZE_BITS * 3;

const LEN_MASK: usize = build_bit_mask(LEN_SHIFT, COMPRESSED_WORD_SIZE);
const LEN_SHIFT: usize = 0;

const RDX_UPPER_MASK: usize = build_bit_mask(RDX_UPPER_SHIFT, usize::BITS as usize / 4 * 1);
const RDX_UPPER_SHIFT: usize = CAP_OFFSET_SHIFT + CAP_OFFSET_BITS;

const RDX_LOWER_MASK: usize = build_bit_mask(usize::BITS as usize / 8 * 5, usize::BITS as usize / 8 * 3);
const RDX_LOWER_SHIFT: usize = RDX_LOWER_MASK.trailing_zeros() as usize;

const WRX_MASK: usize = build_bit_mask(WRX_SHIFT, COMPRESSED_WORD_SIZE);
const WRX_SHIFT: usize = 0;

const OFFSET_MASK: usize = build_bit_mask(OFFSET_SHIFT, COMPRESSED_WORD_SIZE);
const OFFSET_SHIFT: usize = 0;

const CAP_OFFSET_MASK: usize = build_bit_mask(COMPRESSED_WORD_SIZE, CAP_OFFSET_BITS);
const CAP_OFFSET_SHIFT: usize = TAIL_SHIFT;
const CAP_OFFSET_BITS: usize = round_up_pow_2(COMPRESSED_WORD_SIZE - TAIL_SPACE).trailing_zeros() as usize;
const CAP_MASK: usize = TAIL_MASK;
const CAP_SHIFT: usize = TAIL_SHIFT;


pub(crate) const COMPRESSED_WORD_SIZE: usize = usize::BITS as usize / 8 * 5;
pub(crate) const TAIL_SPACE: usize = usize::BITS as usize - COMPRESSED_WORD_SIZE;

pub(crate) const WORD_MASK: usize = build_bit_mask(0, COMPRESSED_WORD_SIZE);
pub(crate) const TAIL_MASK: usize = build_bit_mask(COMPRESSED_WORD_SIZE, TAIL_SPACE);
pub(crate) const TAIL_SHIFT: usize = COMPRESSED_WORD_SIZE;

// returns a pair of capacity and capacity_offset
#[inline]
pub(crate) const fn translate_cap(capacity: usize) -> (usize, usize) {
    let req_bit = round_up_pow_2(capacity).leading_zeros() as usize;
    let offset = req_bit.saturating_sub(TAIL_SPACE);
    let clear_offset = usize::BITS as usize - offset;
    // get the lower bits that normally get cleared.
    let lower_bits = capacity << clear_offset;
    let rounded_cap = greater_zero_ret_one(lower_bits) << (clear_offset + 1);
    ((capacity >> offset) + rounded_cap, offset)
}

// representation on 64 bit systems:
//
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
//
// -> 2 bits remaining!
//
// 1. word: len[40 bits], cap_offset [4 bits], rdx_upper[16 bits], unused[2 bits], flags[2 bits]
// 2. word: wrx[40 bits], rdx_lower[24 bits]
// 3. word: offset[40 bits], capacity[24 bits]

/// This format is slower but allows for a maximum capacity of `size_of(usize) / 8 * 5`.
#[derive(Clone)]
pub struct FormatExtended(BaseBuffer);

impl<const INLINE_SUPPORT: bool, const STATIC_SUPPORT: bool> BufferFormat<INLINE_SUPPORT, STATIC_SUPPORT> for FormatExtended {
    type FlagsTy = BufferTy;

    #[inline]
    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: Self::FlagsTy) -> Self {
        let (cap, cap_offset) = translate_cap(cap);
        let len = len | flags.0 | (cap_offset << CAP_OFFSET_SHIFT);
        Self(BaseBuffer {
            len,
            buffer: BufferUnion { reference: ReferenceBuffer::new(cap, offset, wrx, ptr), },
        })
    }

    #[inline]
    fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self {
        Self(BaseBuffer {
            len: len | (offset << INLINE_OFFSET_SHIFT),
            buffer: BufferUnion { inlined: [0; 3], },
        })
    }

    #[inline]
    fn len_reference(&self) -> usize {
        (self.0.len & LEN_MASK) >> LEN_SHIFT
    }

    #[inline]
    fn len_inlined(&self) -> usize {
        self.0.len & INLINE_LEN_MASK
    }

    #[inline]
    fn set_len_reference(&mut self, len: usize) {
        self.0.len = (self.0.len & !LEN_MASK) | (len << LEN_SHIFT);
    }

    #[inline]
    fn set_len_inlined(&mut self, len: usize) {
        self.0.len = (self.0.len & !INLINE_LEN_MASK) | len;
    }

    #[inline]
    fn offset_reference(&self) -> usize {
        (self.0.buffer.reference.offset & OFFSET_MASK) << OFFSET_SHIFT
    }

    #[inline]
    fn offset_inlined(&self) -> usize {
        (self.0.len & INLINE_OFFSET_MASK) >> INLINE_OFFSET_SHIFT
    }

    #[inline]
    fn set_offset_reference(&mut self, offset: usize) {
        self.0.buffer.reference.offset = (self.0.buffer.reference.offset & !OFFSET_MASK) | (offset << OFFSET_SHIFT);
    }

    #[inline]
    fn set_offset_inlined(&mut self, offset: usize) {
        self.0.len = (self.0.len & !INLINE_OFFSET_MASK) | (offset << INLINE_OFFSET_SHIFT);
    }
    
    #[inline]
    fn wrx_reference(&self) -> usize {
        (self.0.buffer.reference.wrx & WRX_MASK) >> WRX_SHIFT
    }

    #[inline]
    fn wrx_inlined(&self) -> usize {
        (self.0.len & INLINE_WRX_MASK) >> INLINE_WRX_SHIFT
    }

    #[inline]
    fn set_wrx_reference(&mut self, wrx: usize) {
        self.0.buffer.reference.wrx = (self.0.buffer.reference.wrx & !WRX_MASK) | (wrx << WRX_SHIFT);
    }

    #[inline]
    fn set_wrx_inlined(&mut self, wrx: usize) {
        self.0.len = (self.0.len & !INLINE_WRX_MASK) | (wrx << INLINE_WRX_SHIFT);
    }

    #[inline]
    fn rdx_reference(&self) -> usize {
        ((self.0.buffer.reference.wrx & RDX_LOWER_MASK) >> RDX_LOWER_SHIFT) | ((self.0.len & RDX_UPPER_MASK) >> (RDX_UPPER_SHIFT - RDX_LOWER_SHIFT))
    }

    #[inline]
    fn rdx_inlined(&self) -> usize {
        (self.0.len & INLINE_RDX_MASK) >> INLINE_RDX_SHIFT
    }

    #[inline]
    fn set_rdx_reference(&mut self, rdx: usize) {
        self.0.buffer.reference.wrx = (self.0.buffer.reference.wrx & !RDX_LOWER_MASK) | (rdx << RDX_LOWER_SHIFT);
        self.0.len = (self.0.len & !RDX_UPPER_MASK) | ((rdx << (RDX_UPPER_SHIFT - RDX_LOWER_SHIFT)) & RDX_UPPER_MASK);
    }

    #[inline]
    fn set_rdx_inlined(&mut self, rdx: usize) {
        self.0.len = (self.0.len & !INLINE_RDX_MASK) | (rdx << INLINE_RDX_SHIFT);
    }

    #[inline]
    fn cap_reference(&self) -> usize {
        let raw = (self.0.buffer.reference.offset & CAP_MASK) >> CAP_SHIFT;
        let shift = (self.0.len & CAP_OFFSET_MASK) >> CAP_OFFSET_SHIFT;
        raw << shift
    }

    fn cap_inlined(&self) -> usize {
        todo!()
    }

    #[inline]
    fn set_cap_reference(&mut self, cap: usize) {
        let (cap, cap_offset) = translate_cap(cap);
        self.0.buffer.reference.offset = (self.0.buffer.reference.offset & !CAP_MASK) | (cap << CAP_SHIFT);
        self.0.len = (self.0.len & !CAP_OFFSET_MASK) | (cap_offset << CAP_OFFSET_SHIFT);
    }

    fn set_cap_inlined(&mut self, cap: usize) {
        todo!()
    }

    #[inline]
    fn ptr_reference(&self) -> *mut u8 {
        self.0.buffer.reference.ptr
    }

    #[inline]
    fn ptr_inlined(&self) -> *mut u8 {
        (&self.0.buffer.inlined as *const [usize; 3]).cast::<u8>().cast_mut()
    }

    #[inline]
    fn set_ptr_reference(&mut self, ptr: *mut u8) {
        self.0.buffer.reference.ptr = ptr;
    }

    #[inline]
    fn flags(&self) -> Self::FlagsTy {
        BufferTy(self.0.len & BUFFER_TY_MASK)
    }
}

const INLINE_BUFFER_FLAG: usize = 1 << (usize::BITS - 1);
const STATIC_BUFFER_FLAG: usize = 1 << (usize::BITS - 2);

/// the last 2 bits will never be used as allocations are capped at usize::MAX / 8 * 5
const BUFFER_TY_MASK: usize = build_bit_mask(usize::BITS as usize - 2, 2);

#[derive(Clone, Copy, Debug, PartialEq)]
struct BufferTy(usize);

impl Flags for BufferTy {
    #[inline]
    fn new_inlined() -> Self {
        Self(INLINE_BUFFER_FLAG)
    }

    #[inline]
    fn new_static_reference() -> Self {
        Self(STATIC_BUFFER_FLAG)
    }

    #[inline]
    fn new_reference() -> Self {
        Self(0)
    }

    #[inline]
    fn is_inlined(self) -> bool {
        self.0 == INLINE_BUFFER_FLAG
    }

    #[inline]
    fn is_static_reference(self) -> bool {
        self.0 == STATIC_BUFFER_FLAG
    }

    #[inline]
    fn is_reference(self) -> bool {
        self.0 == 0
    }
}
