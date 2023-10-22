use crate::{buffer_format::{BaseBuffer, INLINE_SIZE_BITS}, util::build_bit_mask};

use super::{BufferLayout, Flags};

const INLINE_LEN_MASK: usize = build_bit_mask(0, INLINE_SIZE_BITS);
const INLINE_OFFSET_MASK: usize = build_bit_mask(INLINE_OFFSET_SHIFT, INLINE_SIZE_BITS);
const INLINE_OFFSET_SHIFT: usize = INLINE_SIZE_BITS;
const INLINE_WRX_MASK: usize = build_bit_mask(INLINE_WRX_SHIFT, INLINE_SIZE_BITS);
const INLINE_WRX_SHIFT: usize = INLINE_SIZE_BITS * 2;
const INLINE_RDX_MASK: usize = build_bit_mask(INLINE_RDX_SHIFT, INLINE_SIZE_BITS);
const INLINE_RDX_SHIFT: usize = INLINE_SIZE_BITS * 3;

const RDX_UPPER_MASK: usize = build_bit_mask(RDX_UPPER_SHIFT, usize::BITS as usize / 4 * 1);
const RDX_UPPER_SHIFT: usize = usize::BITS as usize / 4 * 3;

#[derive(Clone)]
pub struct FormatExtended(BaseBuffer);

impl BufferLayout for FormatExtended {
    type FlagsTy;

    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: Self::FlagsTy) -> Self {
        todo!()
    }

    fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self {
        todo!()
    }

    fn len_reference(&self) -> usize {
        todo!()
    }

    fn len_inlined(&self) -> usize {
        todo!()
    }

    fn set_len_reference(&mut self, len: usize) {
        todo!()
    }

    fn set_len_inlined(&mut self, len: usize) {
        todo!()
    }

    fn offset_reference(&self) -> usize {
        todo!()
    }

    fn offset_inlined(&self) -> usize {
        todo!()
    }

    fn set_offset_reference(&mut self, offset: usize) {
        todo!()
    }

    fn set_offset_inlined(&mut self, offset: usize) {
        todo!()
    }

    fn wrx_reference(&self) -> usize {
        todo!()
    }

    fn wrx_inlined(&self) -> usize {
        todo!()
    }

    fn set_wrx_reference(&mut self, wrx: usize) {
        todo!()
    }

    fn set_wrx_inlined(&mut self, wrx: usize) {
        todo!()
    }

    fn rdx_reference(&self) -> usize {
        todo!()
    }

    fn rdx_inlined(&self) -> usize {
        todo!()
    }

    fn set_rdx_reference(&mut self, rdx: usize) {
        todo!()
    }

    fn set_rdx_inlined(&mut self, rdx: usize) {
        todo!()
    }

    fn cap_reference(&self) -> usize {
        todo!()
    }

    fn cap_inlined(&self) -> usize {
        todo!()
    }

    fn set_cap_reference(&mut self, cap: usize) {
        todo!()
    }

    fn set_cap_inlined(&mut self, cap: usize) {
        todo!()
    }

    fn flags(&self) -> Self::FlagsTy {
        todo!()
    }
}


/*

 #[inline]
    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: usize) -> Self {
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

*/

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
