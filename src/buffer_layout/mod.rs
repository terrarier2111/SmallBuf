use std::fmt::Debug;

pub mod half;
pub mod extended;

pub trait BufferLayout: Sized + Clone {

    type FlagsTy: Flags;

    fn new_reference(len: usize, cap: usize, wrx: usize, rdx: usize, offset: usize, ptr: *mut u8, flags: Self::FlagsTy) -> Self;

    fn new_inlined(len: usize, offset: usize, value: [usize; 3]) -> Self;

    fn len_reference(&self) -> usize;

    fn len_inlined(&self) -> usize;
    
    fn set_len_reference(&mut self, len: usize);

    fn set_len_inlined(&mut self, len: usize);

    fn offset_reference(&self) -> usize;

    fn offset_inlined(&self) -> usize;
    
    fn set_offset_reference(&mut self, offset: usize);

    fn set_offset_inlined(&mut self, offset: usize);

    fn wrx_reference(&self) -> usize;

    fn wrx_inlined(&self) -> usize;
    
    fn set_wrx_reference(&mut self, wrx: usize);

    fn set_wrx_inlined(&mut self, wrx: usize);

    fn rdx_reference(&self) -> usize;

    fn rdx_inlined(&self) -> usize;
    
    fn set_rdx_reference(&mut self, rdx: usize);

    fn set_rdx_inlined(&mut self, rdx: usize);

    fn cap_reference(&self) -> usize;

    fn cap_inlined(&self) -> usize;
    
    fn set_cap_reference(&mut self, cap: usize);

    fn set_cap_inlined(&mut self, cap: usize);

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
