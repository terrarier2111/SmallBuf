use std::fmt::Debug;

pub mod half;
pub mod extended;

pub trait BufferFormat<const INLINE_SUPPORT: bool, const STATIC_SUPPORT: bool = true>: Sized + Clone {

    type FlagsTy: Flags;

    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: Self::FlagsTy) -> Self;

    fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self;

    fn len_reference(&self) -> usize;

    fn len_inlined(&self) -> usize;

    #[inline]
    fn len(&self) -> usize {
        if self.flags().is_inlined() {
            self.len_inlined()
        } else {
            self.len_reference()
        }
    }
    
    fn set_len_reference(&mut self, len: usize);

    fn set_len_inlined(&mut self, len: usize);

    #[inline]
    fn set_len(&mut self, len: usize) {
        if self.flags().is_inlined() {
            self.set_len_inlined(len);
        } else {
            self.set_len_reference(len);
        }
    }

    fn offset_reference(&self) -> usize;

    fn offset_inlined(&self) -> usize;

    #[inline]
    fn offset(&self) -> usize {
        if self.flags().is_inlined() {
            self.offset_inlined()
        } else {
            self.offset_reference()
        }
    }
    
    fn set_offset_reference(&mut self, offset: usize);

    fn set_offset_inlined(&mut self, offset: usize);

    #[inline]
    fn set_offset(&mut self, offset: usize) {
        if self.flags().is_inlined() {
            self.offset_inlined()
        } else {
            self.offset_reference()
        }
    }

    fn wrx_reference(&self) -> usize;

    fn wrx_inlined(&self) -> usize;

    #[inline]
    fn wrx(&self) -> usize {
        if self.flags().is_inlined() {
            self.wrx_inlined()
        } else {
            self.wrx_reference()
        }
    }
    
    fn set_wrx_reference(&mut self, wrx: usize);

    fn set_wrx_inlined(&mut self, wrx: usize);

    #[inline]
    fn set_wrx(&mut self, wrx: usize) {
        if self.flags().is_inlined() {
            self.set_wrx_inlined(wrx);
        } else {
            self.set_wrx_reference(wrx);
        }
    }

    fn rdx_reference(&self) -> usize;

    fn rdx_inlined(&self) -> usize;

    #[inline]
    fn rdx(&self) -> usize {
        if self.flags().is_inlined() {
            self.rdx_inlined()
        } else {
            self.rdx_reference()
        }
    }
    
    fn set_rdx_reference(&mut self, rdx: usize);

    fn set_rdx_inlined(&mut self, rdx: usize);

    #[inline]
    fn set_rdx(&mut self, rdx: usize) {
        if self.flags().is_inlined() {
            self.set_rdx_inlined(rdx);
        } else {
            self.set_rdx_reference(rdx);
        }
    }

    fn cap_reference(&self) -> usize;

    fn cap_inlined(&self) -> usize;

    #[inline]
    fn cap(&self) -> usize {
        if self.flags().is_inlined() {
            self.cap_inlined()
        } else {
            self.cap_reference()
        }
    }
    
    fn set_cap_reference(&mut self, cap: usize);

    fn set_cap_inlined(&mut self, cap: usize);

    #[inline]
    fn set_cap(&mut self, cap: usize) {
        if self.flags().is_inlined() {
            self.set_cap_inlined(cap);
        } else {
            self.set_cap_reference(cap);
        }
    }

    fn ptr_reference(&self) -> *mut u8;

    fn ptr_inlined(&self) -> *mut u8;

    #[inline]
    fn ptr(&self) -> *mut u8 {
        if self.flags().is_inlined() {
            self.ptr_inlined()
        } else {
            self.ptr_reference()
        }
    }

    fn set_ptr_reference(&mut self, ptr: *mut u8);

    fn flags(&self) -> Self::FlagsTy;

}

pub trait Flags: Sized + Copy + Clone + Debug + PartialEq {

    fn new_inlined() -> Self;

    fn new_static_reference() -> Self;

    fn new_reference() -> Self;

    fn is_inlined(self) -> bool;

    fn is_static_reference(self) -> bool;

    /// Whether the buffer layout is a non-static reference.
    fn is_reference(self) -> bool;

}
