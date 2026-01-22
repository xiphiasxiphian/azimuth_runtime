// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

use crate::memory::allocators::ALIGNMENT;

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
    pub fn with_capacity(capacity: usize) -> Option<Self>
    {
        let layout = Layout::from_size_align(capacity, ALIGNMENT).ok()?;
        let base = unsafe { alloc(layout) };

        Some(Self {
            base: NonNull::new(base)?,
            capacity,
            layout: Some(layout),
        })
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
