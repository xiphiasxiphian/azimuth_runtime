// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

use crate::memory::allocators::ALIGNMENT;

pub struct GeneralAllocator
{
    base: NonNull<u8>,
    base_block: NonNull<BlockHeader>,
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
        let base = NonNull::new(unsafe { alloc(layout) })?;

        Some(Self {
            base,
            base_block: BlockHeader::get_initial(base, capacity),
            capacity,
            layout: Some(layout),
        })
    }

    pub fn from_existing_allocation(base: NonNull<u8>, capacity: usize) -> Self
    {
        Self {
            base,
            base_block: BlockHeader::get_initial(base, capacity),
            capacity,
            layout: None,
        }
    }

    pub fn raw_alloc(&mut self, size: usize) -> Option<NonNull<u8>>
    {
        todo!()
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        todo!()
    }
}


struct BlockHeader
{
    size: usize,
    free: bool,
    age: u8
}

impl BlockHeader
{
    const ALIGNED_SIZE: usize = size_of::<Self>().next_multiple_of(ALIGNMENT);

    fn get_initial(base: NonNull<u8>, capacity: usize) -> NonNull<Self>
    {
        unsafe { base.cast().write(BlockHeader {
            size: capacity,
            free: true,
            age: 0,
        }) }

        base.cast()
    }

    unsafe fn get_data_pointer<T>(block: NonNull<Self>) -> NonNull<T>
    {
        let initial: NonNull<T> = unsafe { block.byte_add(Self::ALIGNED_SIZE).cast() };
        initial
    }

    unsafe fn next_block(block: NonNull<Self>) -> NonNull<Self>
    {
        let offset = unsafe { block.read().size };
        unsafe { block.byte_add(offset) }
    }

    unsafe fn next_block_checked(block: NonNull<Self>, limit: NonNull<u8>) -> Option<NonNull<Self>>
    {
        let init = unsafe { Self::next_block(block) };
        (init.cast() <= limit).then(|| {
            init
        })
    }
}

#[cfg(test)]
mod general_allocator_tests
{
    use super::*;

    #[test]
    fn create_allocator()
    {
        let _ = GeneralAllocator::with_capacity(1024).unwrap();
    }

    #[test]
    fn create_from_existing()
    {
        let mut base = unsafe { Box::<[u8]>::new_zeroed_slice(1024).assume_init() };
        let allocator = GeneralAllocator::from_existing_allocation(NonNull::new(base.as_mut_ptr()).unwrap(), 512);

        // Maybe test some allocations here
    }
}
