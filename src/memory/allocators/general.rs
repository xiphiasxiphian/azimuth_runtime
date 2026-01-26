// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

use crate::{common::{self, ScopeMethods}, guard, memory::allocators::{ALIGNMENT, AllocatorError, MIN_PAGE_ALIGNMENT}};

pub struct GeneralAllocator<const DEPTH: usize>
{
    base: NonNull<u8>,
    capacity: usize,
    freelists: [*mut BlockHeader; DEPTH],
    min_block_size: usize,
    layout: Option<Layout>,
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

        guard!(base.as_ptr() as usize & (MIN_PAGE_ALIGNMENT - 1) == 0, AllocatorError::BadConstraints);
        guard!(capacity >= min_block_size, AllocatorError::BadConstraints);
        guard!(min_block_size >= size_of::<BlockHeader>(), AllocatorError::BadConstraints);
        guard!(capacity.is_power_of_two(), AllocatorError::BadConstraints);

        let freelists: [*mut BlockHeader; DEPTH]
            = [std::ptr::null_mut(); DEPTH].also_mut(|x| { x[DEPTH - 1] = base.as_ptr() as *mut BlockHeader} );

        Ok(Self {
            base,
            capacity,
            freelists,
            min_block_size,
            layout,
        })
    }

    pub fn with_capacity(capacity: usize) -> Result<Self, AllocatorError>
    {
        let layout = Layout::from_size_align(capacity, ALIGNMENT)
            .map_err(|x| AllocatorError::BadLayout(x))?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(AllocatorError::FailedInitialAllocation)?;

        Self::new(base, capacity, Some(layout))
    }

    pub fn from_existing_allocation(base: NonNull<u8>, capacity: usize) -> Result<Self, AllocatorError>
    {
        Self::new(base, capacity, None)
    }

    pub fn raw_alloc(&mut self, size: usize) -> Option<NonNull<u8>>
    {
        todo!()
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        todo!()
    }


    fn get_allocation_size(&self, in_size: usize, alignment: usize) -> Result<usize, AllocatorError>
    {
        guard!(alignment.is_power_of_two(), AllocatorError::BadRequest);
        guard!(alignment <= MIN_PAGE_ALIGNMENT, AllocatorError::BadRequest);

        let mut size = in_size;

        if alignment > size { size = alignment; }
        size = size.max(self.min_block_size).next_power_of_two();

        guard!(size <= self.capacity, AllocatorError::BadRequest);
        Ok(size)
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
        let _ = GeneralAllocator::<16>::with_capacity(4096).unwrap();
    }

    #[test]
    fn create_from_existing()
    {
        let mut base = unsafe { Box::<[u8]>::new_zeroed_slice(4096).assume_init() };
        let allocator = GeneralAllocator::<16>::from_existing_allocation(NonNull::new(base.as_mut_ptr()).unwrap(), 512);

        // Maybe test some allocations here
    }
}
