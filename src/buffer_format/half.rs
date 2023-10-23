use crate::{buffer_layout::{BaseBuffer, INLINE_SIZE_BITS, BufferUnion, ReferenceBuffer}, util::build_bit_mask};

use super::{BufferFormat, Flags, extended::translate_cap};

const INLINE_LEN_MASK: usize = build_bit_mask(0, INLINE_SIZE_BITS);
const INLINE_OFFSET_MASK: usize = build_bit_mask(INLINE_OFFSET_SHIFT, INLINE_SIZE_BITS);
const INLINE_OFFSET_SHIFT: usize = INLINE_SIZE_BITS;
const INLINE_WRX_MASK: usize = build_bit_mask(INLINE_WRX_SHIFT, INLINE_SIZE_BITS);
const INLINE_WRX_SHIFT: usize = INLINE_SIZE_BITS * 2;
const INLINE_RDX_MASK: usize = build_bit_mask(INLINE_RDX_SHIFT, INLINE_SIZE_BITS);
const INLINE_RDX_SHIFT: usize = INLINE_SIZE_BITS * 3;

const LEN_MASK: usize = build_bit_mask(LEN_SHIFT, usize::BITS as usize / 2);
const LEN_SHIFT: usize = 0;

const RDX_MASK: usize = build_bit_mask(RDX_SHIFT, usize::BITS as usize / 2);
const RDX_SHIFT: usize = usize::BITS as usize / 2;

const WRX_MASK: usize = build_bit_mask(WRX_SHIFT, usize::BITS as usize / 2);
const WRX_SHIFT: usize = 0;

const CAP_MASK: usize = build_bit_mask(CAP_SHIFT, usize::BITS as usize / 2);
const CAP_SHIFT: usize = usize::BITS as usize / 2;

const OFFSET_MASK: usize = build_bit_mask(OFFSET_SHIFT, usize::BITS as usize / 2);
const OFFSET_SHIFT: usize = 0;

// representation on 64 bit systems:
//
// total available bits: 64 * 3 = 192
// writer index: 32
// reader index: 32
// offset: 32
// len: 32
// flags: 2
// 192 - 32 - 32 - 32 - 32 - 2 = 62
// capacity: 32
// 62 - 32 = 30
//
// -> 30 bits remaining!
//
// 1. word: len[32 bits], unused[30 bits], flags[2 bits]
// 2. word: wrx[32 bits], rdx[32 bits]
// 3. word: offset[32 bits], capacity[32 bits]

/// This format is faster but only allows for a maximum capacity of `size_of(usize) / 2`.
#[derive(Clone)]
pub struct FormatHalf(BaseBuffer);

impl<const INLINE_SUPPORT: bool, const STATIC_SUPPORT: bool> BufferFormat<INLINE_SUPPORT, STATIC_SUPPORT> for FormatHalf {
    type FlagsTy = BufferTy<INLINE_SUPPORT, STATIC_SUPPORT>;

    #[inline]
    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: Self::FlagsTy) -> Self {
        let (cap, cap_offset) = translate_cap(cap);
        let len = len | flags.0;
        Self(BaseBuffer {
            len,
            buffer: BufferUnion { reference: ReferenceBuffer::new(cap, offset, wrx, ptr), },
        })
    }

    #[inline]
    fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        Self(BaseBuffer {
            len: len | (offset << INLINE_OFFSET_SHIFT),
            buffer: BufferUnion { inlined: [0; 3], },
        })
    }

    #[inline]
    fn len_reference(&self) -> usize {
        self.0.len & LEN_MASK
    }

    #[inline]
    fn len_inlined(&self) -> usize {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        self.0.len & INLINE_LEN_MASK
    }

    #[inline]
    fn set_len_reference(&mut self, len: usize) {
        self.0.len = (self.0.len & !LEN_MASK) | (len << LEN_SHIFT);
    }

    #[inline]
    fn set_len_inlined(&mut self, len: usize) {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        self.0.len = (self.0.len & !INLINE_LEN_MASK) | len;
    }

    #[inline]
    fn offset_reference(&self) -> usize {
        (self.0.buffer.reference.offset & OFFSET_MASK) >> OFFSET_SHIFT
    }

    #[inline]
    fn offset_inlined(&self) -> usize {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        (self.0.len & INLINE_OFFSET_MASK) >> INLINE_OFFSET_SHIFT
    }

    #[inline]
    fn set_offset_reference(&mut self, offset: usize) {
        self.0.buffer.reference.offset = (self.0.buffer.reference.offset & !OFFSET_MASK) | (offset << OFFSET_SHIFT);
    }

    #[inline]
    fn set_offset_inlined(&mut self, offset: usize) {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        self.0.len = (self.0.len & !INLINE_OFFSET_MASK) | (offset << INLINE_OFFSET_SHIFT);
    }
    
    #[inline]
    fn wrx_reference(&self) -> usize {
        (self.0.buffer.reference.wrx & WRX_MASK) >> WRX_SHIFT
    }

    #[inline]
    fn wrx_inlined(&self) -> usize {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        (self.0.len & INLINE_WRX_MASK) >> INLINE_WRX_SHIFT
    }

    #[inline]
    fn set_wrx_reference(&mut self, wrx: usize) {
        self.0.buffer.reference.wrx = (self.0.buffer.reference.wrx & !WRX_MASK) | (wrx << WRX_SHIFT);
    }

    #[inline]
    fn set_wrx_inlined(&mut self, wrx: usize) {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        self.0.len = (self.0.len & !INLINE_WRX_MASK) | (wrx << INLINE_WRX_SHIFT);
    }

    #[inline]
    fn rdx_reference(&self) -> usize {
        (self.0.buffer.reference.wrx & RDX_MASK) >> RDX_SHIFT
    }

    #[inline]
    fn rdx_inlined(&self) -> usize {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        (self.0.len & INLINE_RDX_MASK) >> INLINE_RDX_SHIFT
    }

    #[inline]
    fn set_rdx_reference(&mut self, rdx: usize) {
        self.0.buffer.reference.wrx = (self.0.buffer.reference.wrx & !RDX_MASK) | (rdx << RDX_SHIFT);
    }

    #[inline]
    fn set_rdx_inlined(&mut self, rdx: usize) {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        self.0.len = (self.0.len & !INLINE_RDX_MASK) | (rdx << INLINE_RDX_SHIFT);
    }

    #[inline]
    fn cap_reference(&self) -> usize {
        (self.0.buffer.reference.offset & CAP_MASK) >> CAP_SHIFT
    }

    fn cap_inlined(&self) -> usize {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        todo!()
    }

    #[inline]
    fn set_cap_reference(&mut self, cap: usize) {
        self.0.buffer.reference.offset = (self.0.buffer.reference.offset & !CAP_MASK) | (cap << CAP_SHIFT);
    }

    fn set_cap_inlined(&mut self, cap: usize) {
        if !INLINE_SUPPORT {
            unreachable!();
        }
        todo!()
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
struct BufferTy<const SUPPORT_INLINE: bool, const SUPPORT_STATIC: bool = true>(usize);

impl<const SUPPORT_INLINE: bool, const SUPPORT_STATIC: bool> Flags for BufferTy<SUPPORT_INLINE, SUPPORT_STATIC> {
    #[inline]
    fn new_inlined() -> Self {
        if !SUPPORT_INLINE {
            unreachable!();
        }
        Self(INLINE_BUFFER_FLAG)
    }

    #[inline]
    fn new_static_reference() -> Self {
        if !SUPPORT_STATIC {
            unreachable!();
        }
        Self(STATIC_BUFFER_FLAG)
    }

    #[inline]
    fn new_reference() -> Self {
        Self(0)
    }

    #[inline]
    fn is_inlined(self) -> bool {
        SUPPORT_INLINE && self.0 == INLINE_BUFFER_FLAG
    }

    #[inline]
    fn is_static_reference(self) -> bool {
        SUPPORT_STATIC && self.0 == STATIC_BUFFER_FLAG
    }

    #[inline]
    fn is_reference(self) -> bool {
        (!SUPPORT_INLINE && !SUPPORT_STATIC) || self.0 == 0
    }
}
