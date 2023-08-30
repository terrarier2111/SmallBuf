#![feature(generic_const_exprs)]

use std::borrow::Borrow;
use std::ops::Deref;

pub mod buffer;
pub mod buffer_rw;
pub mod buffer_mut;
mod util;

pub trait GenericBuffer: Clone + AsRef<[u8]> + Deref<Target = [u8]> + Borrow<[u8]> + Into<Vec<u8>> + From<Vec<u8>> {

    fn new() -> Self;

    fn len(&self) -> usize;

    fn capacity(&self) -> usize;

    fn clear(&mut self);

    /// tries to shrink the backing allocation down to a value that at least fits the current
    /// buffer, but may be larger than that as this operation operates on a best-effort basis.
    fn shrink(&mut self);

}

pub trait ReadableBuffer: GenericBuffer + From<&'static [u8]> {

    #[inline]
    fn from_static(buf: &'static [u8]) -> Self {
        <Self as From<&'static [u8]>>::from(buf)
    }

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
    use crate::buffer_mut::{BufferMut, BufferMutGeneric};
    use crate::{GenericBuffer, ReadableBuffer, WritableBuffer};
    use crate::buffer::{Buffer, BufferGeneric};
    use crate::buffer_rw::BufferRW;

    #[test]
    fn test_buffer_mut() {
        let mut buffer = BufferMut::new();
        buffer.put_u64_le(8);
        assert_eq!(buffer.capacity(), size_of::<usize>() * 2);
        assert_eq!(buffer.len(), 8);
        buffer.put_u64_le(7);
        buffer.put_u16_le(1);
        buffer.put_u8(5);
        assert_eq!(buffer.len(), 19);
        let buffer_2 = buffer.clone();
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        let converted = Buffer::from(buffer_2.clone());
        assert_eq!(converted.len(), buffer_2.len());
        assert!(converted.len() > 0);
        assert!(converted.capacity() > 0);
        let mut cloned = converted.clone();
        assert_eq!(cloned.len(), converted.len());
        assert_eq!(cloned.capacity(), converted.capacity());

        assert_eq!(cloned.get_u64_le(), 8);
        assert_eq!(cloned.get_u64_le(), 7);
        assert_eq!(cloned.get_u16_le(), 1);
        assert_eq!(cloned.get_u8(), 5);

        let mut buffer = BufferRW::from(cloned);
        assert_eq!(buffer.len(), 19);
        buffer.put_u64_le(5);
        assert_eq!(buffer.get_u64_le(), 5);
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
    }

}

#[derive(PartialEq, Eq)]
pub struct BufferCfg {
    pub inline_small: bool,
    pub static_storage: bool,
    pub fast_conversion: bool, // this has to be at least 2
    pub growth_factor: usize,
}

impl BufferCfg {

    #[inline]
    pub const fn new() -> Self {
        Self {
            inline_small: true,
            static_storage: true,
            fast_conversion: true,
            growth_factor: 2,
        }
    }

}

impl Default for BufferCfg {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
