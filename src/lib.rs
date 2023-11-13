use std::borrow::Borrow;
use std::ops::{Deref, RangeBounds};

pub mod buffer;
pub mod buffer_rw;
pub mod buffer_mut;
mod util;
mod buffer_layout;
mod buffer_format;

pub trait GenericBuffer: Clone + AsRef<[u8]> + Deref<Target = [u8]> + Borrow<[u8]> + Into<Vec<u8>> + From<Vec<u8>> {

    /// creates a new empty instance of a buffer
    fn new() -> Self;

    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize;

    fn clear(&mut self);

    /// tries to shrink the backing allocation down to a value that at least fits the current
    /// buffer, but may be larger than that as this operation operates on a best-effort basis.
    fn shrink(&mut self);

    fn truncate(&mut self, len: usize);

    /// this will split off everything past offset bytes from the current
    /// [type-dependent] index, returning the split-off buffer.
    /// 
    /// [type-dependent]: This is reader for `buffer.rs` and `buffer_rw.rs` and writer for `buffer_mut.rs`
    ///
    /// offset represents an offset from the current reader index
    fn split_off(&mut self, offset: usize) -> Self;

    /// splits off everything before the current [type-dependent] index offset by
    /// the offset parameter, returning the split-off buffer.
    /// 
    /// [type-dependent]: This is reader for `buffer.rs` and `buffer_rw.rs` and writer for `buffer_mut.rs`
    ///
    /// offset represents an offset from the current reader/writer index
    fn split_to(&mut self, offset: usize) -> Self;

    /// this will split the current view at the current [type-dependent] index,
    /// leaving the current buffer' remaining empty.
    /// 
    /// [type-dependent]: This is reader for `buffer.rs` and `buffer_rw.rs` and writer for `buffer_mut.rs`
    /// 
    fn split(&mut self) -> Self;

    /// this will merge the current view and another view that was previously split off
    /// from it. Note that this will only work on buffers that weren't modified in a way
    /// that caused reallocation after splitting. Note, that the inactive index
    /// (opposite of the type-dependent index specified above) will be reset to the beginning of the comboned buffer.
    /// 
    /// #panic
    /// panics on failure to merge buffers as described above.
    fn unsplit(&mut self, other: Self);

    /// this will merge the current view and another view that was previously split off
    /// from it. Note that this will only work on buffers that weren't modified in a way
    /// that caused reallocation after splitting. Note, that the inactive index
    /// (opposite of the type-dependent index specified above) will be reset to the beginning of the comboned buffer.
    fn try_unsplit(&mut self, other: Self) -> Result<(), Self>;

}

pub trait ReadableBuffer: GenericBuffer + From<&'static [u8]> {

    #[inline]
    fn from_static(buf: &'static [u8]) -> Self {
        <Self as From<&'static [u8]>>::from(buf)
    }

    /// Resets the reader index to 0 so the buffer can be read
    /// from the beginning again.
    fn reset_reader_index(&mut self);

    /// Advanced the reader index by `amount`
    /// 
    /// #Panic
    /// May panic when amount exceeds `remaining()`.
    fn advance(&mut self, amount: usize);

    /// this will return the amount of remaining bytes that can be
    /// read from this buffer
    fn remaining(&self) -> usize;

    fn get_slice(&mut self, bytes: usize) -> &[u8];

    #[inline]
    fn get_slice_bound<const LEN: usize>(&mut self) -> [u8; LEN] {
        let src = self.get_slice(LEN);
        let mut ret = [0; LEN];
        for i in 0..LEN {
            ret[i] = unsafe { *src.get_unchecked(i) };
        }
        ret
    }

    fn get_u8(&mut self) -> u8;

    #[inline]
    fn get_u16_le(&mut self) -> u16 {
        let bytes = self.get_slice_bound::<2>();
        u16::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u16_be(&mut self) -> u16 {
        let bytes = self.get_slice_bound::<2>();
        u16::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u16_ne(&mut self) -> u16 {
        let bytes = self.get_slice_bound::<2>();
        u16::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u32_le(&mut self) -> u32 {
        let bytes = self.get_slice_bound::<4>();
        u32::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u32_be(&mut self) -> u32 {
        let bytes = self.get_slice_bound::<4>();
        u32::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u32_ne(&mut self) -> u32 {
        let bytes = self.get_slice_bound::<4>();
        u32::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u64_le(&mut self) -> u64 {
        let bytes = self.get_slice_bound::<8>();
        u64::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u64_be(&mut self) -> u64 {
        let bytes = self.get_slice_bound::<8>();
        u64::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u64_ne(&mut self) -> u64 {
        let bytes = self.get_slice_bound::<8>();
        u64::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u128_le(&mut self) -> u128 {
        let bytes = self.get_slice_bound::<16>();
        u128::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u128_be(&mut self) -> u128 {
        let bytes = self.get_slice_bound::<16>();
        u128::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u128_ne(&mut self) -> u128 {
        let bytes = self.get_slice_bound::<16>();
        u128::from_ne_bytes(bytes)
    }

}

pub trait WritableBuffer: GenericBuffer {

    /// Allocates a buffer with at least `capacity` bytes of capacity
    /// to store information.
    fn with_capacity(capacity: usize) -> Self;

    /// Allocates a zero initialized buffer with at least `len` bytes capacity
    /// to store information.
    fn zeroed(len: usize) -> Self;

    /// Resets the writer index to 0, so the buffer
    /// can be written again from the start
    fn reset_writer_index(&mut self);

    fn reserve(&mut self, size: usize);

    fn resize(&mut self, size: usize);

    fn capacity(&self) -> usize;

    fn put_bytes(&mut self, val: u8, repeat: usize);

    fn put_slice(&mut self, val: &[u8]);

    fn put_u8(&mut self, val: u8);

    #[inline]
    fn put_u16_le(&mut self, val: u16) {
        let raw = val.to_le_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u16_be(&mut self, val: u16) {
        let raw = val.to_be_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u16_ne(&mut self, val: u16) {
        let raw = val.to_ne_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u32_le(&mut self, val: u32) {
        let raw = val.to_le_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u32_be(&mut self, val: u32) {
        let raw = val.to_be_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u32_ne(&mut self, val: u32) {
        let raw = val.to_ne_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u64_le(&mut self, val: u64) {
        let raw = val.to_le_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u64_be(&mut self, val: u64) {
        let raw = val.to_be_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u64_ne(&mut self, val: u64) {
        let raw = val.to_ne_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u128_le(&mut self, val: u128) {
        let raw = val.to_le_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u128_be(&mut self, val: u128) {
        let raw = val.to_be_bytes();
        self.put_slice(&raw);
    }

    #[inline]
    fn put_u128_ne(&mut self, val: u128) {
        let raw = val.to_ne_bytes();
        self.put_slice(&raw);
    }

}

pub trait ReadonlyBuffer: ReadableBuffer {

    /// the range represents a range offset from the current reader
    /// index.
    fn slice(&self, range_offset: impl RangeBounds<usize>) -> Self;

}

pub trait RWBuffer: ReadableBuffer + WritableBuffer {}

mod tests {
    use std::mem::size_of;
    use crate::buffer_format::BufferFormat;
    use crate::buffer_mut::BufferMut;
    use crate::{GenericBuffer, ReadableBuffer, WritableBuffer};
    use crate::buffer::Buffer;
    use crate::buffer_rw::BufferRW;

    #[test]
    fn test_buffer_mut() {
        let mut buffer = BufferMut::new();
        buffer.put_u8(2);
        buffer.put_u64_le(8);
        assert_eq!(buffer.capacity(), size_of::<usize>() * 3);
        assert_eq!(buffer.len(), 9);
        buffer.put_u64_le(7);
        buffer.put_u16_le(1);
        buffer.put_u64_le(45);
        assert_eq!(buffer.len(), 27);
        assert!(!buffer.is_inlined());
        println!("meta ptr: {}", unsafe { buffer.meta_ptr() } as usize);
        if unsafe { buffer.is_only() } {
            println!("only!");
        }
        let buffer_2 = buffer.clone();
        println!("capacity buf2: {}", buffer_2.capacity());
        assert!(!buffer_2.is_inlined());
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        if unsafe { buffer_2.is_only() } {
            println!("only!");
        }

        let mut converted = Buffer::from(buffer_2.clone());
        assert_eq!(converted.len(), buffer_2.len());
        assert!(converted.len() > 0);
        assert!(converted.capacity() > 0);
        let mut cloned = converted.clone();
        println!("base ptr: {}", cloned.buffer.ptr as usize);
        assert_eq!(cloned.len(), converted.len());
        assert_eq!(cloned.capacity(), converted.capacity());

        assert_eq!(cloned.get_u8(), 2);
        assert_eq!(cloned.get_u64_le(), 8);
        assert_eq!(cloned.get_u64_le(), 7);
        assert_eq!(cloned.get_u16_le(), 1);
        assert_eq!(cloned.get_u64_le(), 45);

        let mut buffer = BufferRW::from(cloned);
        assert_eq!(buffer.len(), 27);
        buffer.put_u64_le(5);
        assert_eq!(buffer.get_u64_le(), 5);
        let mut rw_buf: BufferRW = buffer.into();
        assert_eq!(rw_buf.len(), 35);
        rw_buf.put_u64_le(3);
        rw_buf.shrink();
        assert_eq!(rw_buf.get_u64_le(), 3);
    }

    #[test]
    fn test_static() {
        static BUFFER: &[u8] = &[56, 2, 8, 46, 15, 9];
        let mut buffer = Buffer::from_static(BUFFER);
        assert_eq!(buffer.len(), BUFFER.len());
        assert_eq!(buffer.get_u8(), BUFFER[0]);
        assert_eq!(buffer.get_u8(), BUFFER[1]);
        assert_eq!(buffer.get_u8(), BUFFER[2]);
        assert_eq!(buffer.get_u8(), BUFFER[3]);
        assert_eq!(buffer.get_u8(), BUFFER[4]);
        assert_eq!(buffer.get_u8(), BUFFER[5]);
        let mut buffer = BufferRW::from(buffer);
        buffer.put_u64_le(5);
        assert_eq!(buffer.get_u64_le(), 5);
        // let mut buf_mut = BufferMut::from(buffer); // FIXME: figure out why this impl isn't there
        // buf_mut.put_u8(3);
    }

    #[test]
    fn test_split() {
        let mut buffer = BufferMut::new();
        buffer.put_u64_le(3);
        buffer.put_u128_be(52);
        let original_len = buffer.len();
        let mut buffer = Buffer::from(buffer);
        let mut other = buffer.split_off(9);
        println!("other: len {} rdx {}", other.len(), other.0.rdx());
        println!("buffer: len {} rdx {}", buffer.len(), buffer.0.rdx());
        assert_eq!(other.remaining() + buffer.remaining(), original_len);
        other.unsplit(buffer);
        assert_eq!(other.remaining(), original_len);
    }

}