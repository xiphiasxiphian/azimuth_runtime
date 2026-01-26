// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, LayoutError, alloc, dealloc}, array, ptr::NonNull};

use crate::memory::allocators::{ALIGNMENT, AllocatorError, MIN_PAGE_ALIGNMENT};

pub struct GeneralAllocator<const DEPTH: usize>
{
    base: NonNull<u8>,
    capacity: usize,
    freelists: [*mut BlockHeader; DEPTH],
    layout: Option<Layout>
}

impl<const N: usize> Drop for GeneralAllocator<N>
{
    fn drop(&mut self)
    {
        if let Some(layout) = self.layout { unsafe { dealloc(self.base.as_ptr(), layout); } }
    }
}

impl<const DEPTH: usize> GeneralAllocator<DEPTH>
{
    fn new(base: NonNull<u8>, capacity: usize, layout: Option<Layout>) -> Result<Self, AllocatorError>
    {
        let min_block_size = capacity >> (DEPTH - 1);

        if base.as_ptr() as usize & (MIN_PAGE_ALIGNMENT - 1) != 0
        {
            return Err(AllocatorError::BadConstraints);
        }

        if capacity < min_block_size
        {
            return Err(AllocatorError::BadConstraints);
        }

        if min_block_size < size_of::<BlockHeader>()
        {
            return Err(AllocatorError::BadConstraints);
        }

        if !capacity.is_power_of_two()
        {
            return Err(AllocatorError::BadConstraints);
        }

        let freelists: [*mut BlockHeader; DEPTH] = [std::ptr::null_mut(); DEPTH];


    }

    pub fn with_capacity(capacity: usize) -> Result<Self, AllocatorError>
    {
        let layout = Layout::from_size_align(capacity, ALIGNMENT)
            .map_err(|x| AllocatorError::BadLayout(x))?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(AllocatorError::FailedInitialAllocation)?;

        Self::new::<DEPTH>(base, capacity, Some(layout))
    }

    pub fn from_existing_allocation(base: NonNull<u8>, capacity: usize) -> Self
    {

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
    next: Option<NonNull<Self>>
}

impl BlockHeader
{

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
