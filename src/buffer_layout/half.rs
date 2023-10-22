const INLINE_LEN_MASK: usize = build_bit_mask(0, INLINE_SIZE_BITS);
const INLINE_OFFSET_MASK: usize = build_bit_mask(INLINE_OFFSET_SHIFT, INLINE_SIZE_BITS);
const INLINE_OFFSET_SHIFT: usize = INLINE_SIZE_BITS;
const INLINE_WRX_MASK: usize = build_bit_mask(INLINE_WRX_SHIFT, INLINE_SIZE_BITS);
const INLINE_WRX_SHIFT: usize = INLINE_SIZE_BITS * 2;
const INLINE_RDX_MASK: usize = build_bit_mask(INLINE_RDX_SHIFT, INLINE_SIZE_BITS);
const INLINE_RDX_SHIFT: usize = INLINE_SIZE_BITS * 3;

const RDX_UPPER_MASK: usize = build_bit_mask(RDX_UPPER_SHIFT, usize::BITS as usize / 4 * 1);
const RDX_UPPER_SHIFT: usize = usize::BITS as usize / 4 * 3;