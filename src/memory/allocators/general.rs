// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, dealloc}, ptr::NonNull};

pub struct GeneralAllocator
{
    base: NonNull<u8>,
    layout: Layout
}

impl Drop for GeneralAllocator
{
    fn drop(&mut self)
    {
        unsafe { dealloc(self.base.as_ptr(), self.layout); }
    }
}
