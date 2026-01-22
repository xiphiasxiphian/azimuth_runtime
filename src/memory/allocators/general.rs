// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, dealloc}, ptr::NonNull};

pub struct GeneralAllocator
{
    base: NonNull<u8>,
    capacity: usize,
    layout: Option<Layout>
}

impl Drop for GeneralAllocator
{
    fn drop(&mut self)
    {
        if let Some(layout) = self.layout { unsafe { dealloc(self.base.as_ptr(), layout); } }
    }
}

impl GeneralAllocator
{
    pub fn with_capacity()
    {

    }

    pub fn with_existing_allocation(base: NonNull<u8>, capacity: usize) -> Self
    {
        Self {
            base,
            capacity,
            layout: None,
        }
    }
}
