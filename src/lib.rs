pub mod buffer;
pub mod buffer_rw;
pub mod buffer_mut;
mod util;

pub trait GenericBuffer: Clone + AsRef<[u8]> {

    fn new() -> Self;

    fn len(&self) -> usize;

    fn capacity(&self) -> usize;

    fn clear(&mut self);

    /// tries to shrink the backing allocation down to a value that at least fits the current
    /// buffer, but may be larger than that as this operation operates on a best-effort basis.
    fn shrink(&mut self);

}

pub trait ReadableBuffer: GenericBuffer {

    fn remaining(&self) -> usize;

    fn get_bytes(&mut self, bytes: usize) -> &[u8];

    #[inline]
    fn get_bytes_bound<const LEN: usize>(&mut self) -> [u8; LEN] {
        let src = self.get_bytes(LEN);
        let mut ret = [0; LEN];
        for i in 0..LEN {
            ret[i] = unsafe { *src.get_unchecked(i) };
        }
        ret
    }

    fn get_u8(&mut self) -> u8;

    #[inline]
    fn get_u16_le(&mut self) -> u16 {
        let bytes = self.get_bytes_bound::<2>();
        u16::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u16_be(&mut self) -> u16 {
        let bytes = self.get_bytes_bound::<2>();
        u16::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u16_ne(&mut self) -> u16 {
        let bytes = self.get_bytes_bound::<2>();
        u16::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u32_le(&mut self) -> u32 {
        let bytes = self.get_bytes_bound::<4>();
        u32::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u32_be(&mut self) -> u32 {
        let bytes = self.get_bytes_bound::<4>();
        u32::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u32_ne(&mut self) -> u32 {
        let bytes = self.get_bytes_bound::<4>();
        u32::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u64_le(&mut self) -> u64 {
        let bytes = self.get_bytes_bound::<8>();
        u64::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u64_be(&mut self) -> u64 {
        let bytes = self.get_bytes_bound::<8>();
        u64::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u64_ne(&mut self) -> u64 {
        let bytes = self.get_bytes_bound::<8>();
        u64::from_ne_bytes(bytes)
    }

    #[inline]
    fn get_u128_le(&mut self) -> u128 {
        let bytes = self.get_bytes_bound::<16>();
        u128::from_le_bytes(bytes)
    }

    #[inline]
    fn get_u128_be(&mut self) -> u128 {
        let bytes = self.get_bytes_bound::<16>();
        u128::from_be_bytes(bytes)
    }

    #[inline]
    fn get_u128_ne(&mut self) -> u128 {
        let bytes = self.get_bytes_bound::<16>();
        u128::from_ne_bytes(bytes)
    }

}

pub trait WritableBuffer: GenericBuffer {

    fn with_capacity(capacity: usize) -> Self;

    fn zeroed(len: usize) -> Self;

    fn put_bytes(&mut self, val: &[u8]);

    fn put_u8(&mut self, val: u8);

    #[inline]
    fn put_u16_le(&mut self, val: u16) {
        let raw = val.to_le_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u16_be(&mut self, val: u16) {
        let raw = val.to_be_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u16_ne(&mut self, val: u16) {
        let raw = val.to_ne_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u32_le(&mut self, val: u32) {
        let raw = val.to_le_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u32_be(&mut self, val: u32) {
        let raw = val.to_be_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u32_ne(&mut self, val: u32) {
        let raw = val.to_ne_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u64_le(&mut self, val: u64) {
        let raw = val.to_le_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u64_be(&mut self, val: u64) {
        let raw = val.to_be_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u64_ne(&mut self, val: u64) {
        let raw = val.to_ne_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u128_le(&mut self, val: u128) {
        let raw = val.to_le_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u128_be(&mut self, val: u128) {
        let raw = val.to_be_bytes();
        self.put_bytes(&raw);
    }

    #[inline]
    fn put_u128_ne(&mut self, val: u128) {
        let raw = val.to_ne_bytes();
        self.put_bytes(&raw);
    }

}

pub trait RWBuffer: ReadableBuffer + WritableBuffer {}

mod tests {
    use std::mem::size_of;
    use crate::buffer_mut::BufferMut;
    use crate::{GenericBuffer, WritableBuffer};

    #[test]
    fn test_buffer_mut() {
        let mut buffer = BufferMut::new();
        buffer.put_u64_le(8);
        assert_eq!(buffer.capacity(), size_of::<usize>() * 2);
        assert_eq!(buffer.len(), 8);
        buffer.put_u64_le(7);
        buffer.put_u16_le(1);
        assert_eq!(buffer.len(), 18);
    }

}
