// A memory manager manages a block of memory as a heap

use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

use crate::{common::{ScopeMethods}, guard, memory::allocators::{ALIGNMENT, AllocatorError, MIN_PAGE_ALIGNMENT}};

pub struct GeneralAllocator<const DEPTH: usize>
{
    base: NonNull<u8>,
    capacity: usize,
    freelists: [Option<NonNull<BlockHeader>>; DEPTH],
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

        let freelists = [None; DEPTH].also_mut(|x| { x[DEPTH - 1] = Some(base.cast())} );

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

    pub fn raw_alloc(&mut self, size: usize, align: usize) -> Option<NonNull<u8>>
    {
        self.get_allocation_order(size, align)
            .map(|target| {
                (target..DEPTH)
                    .map(|order| {
                        self.block_pop(order)
                            .inspect(|block| {
                                if order > target
                                {
                                    unsafe { self.split_block(*block, order, target); }
                                }
                            })
                    })
                    .find(|x| x.is_some())
                    .flatten()
            })
            .unwrap_or(None)
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        self.raw_alloc(size_of_val(&value), align_of_val(&value))
            .map(|x| x.cast())
            .inspect(|x| unsafe { x.write(value) })
    }

    pub fn raw_dealloc(&mut self, ptr: NonNull<u8>, size: usize, align: usize)
    {
        let initial = self.get_allocation_order(size, align).expect("Invalid Block Deallocation Request");

        let mut block = ptr;
        for order in initial..DEPTH
        {
            if let Some(buddy) = self.find_buddy(order, block)
            {
                if self.block_remove(order, block)
                {
                    block = block.min(buddy);
                    continue;
                }
            }

            self.block_insert(order, block);
            return;
        }
    }

    pub fn dealloc<T>(&mut self, ptr: NonNull<T>)
    {
        self.raw_dealloc(ptr.cast(), size_of::<T>(), align_of::<T>());
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

    fn get_allocation_order(&self, size: usize, align: usize) -> Result<usize, AllocatorError>
    {
        self.get_allocation_size(size, align)
            .map(|x| (x.ilog2() - self.min_block_size.ilog2()) as usize)
    }

    const fn get_required_block_size(&self, order: usize) -> usize
    {
        1 << (self.min_block_size.ilog2() as usize + order)
    }

    fn block_pop(&mut self, order: usize) -> Option<NonNull<u8>>
    {
        self.freelists[order]
            .inspect(|blk| {
                // Alter free list
                if order != self.freelists.len() - 1
                {
                    self.freelists[order] = unsafe { blk.read().next }
                }
                else
                {
                    self.freelists[order] = None
                }
            })
            .map(|x| x.cast())
    }

    fn block_insert(&mut self, order: usize, block: NonNull<u8>)
    {
        let new_head = block.cast();
        unsafe { new_head.write(BlockHeader { next: self.freelists[order] }); }

        self.freelists[order] = Some(new_head);
    }

    fn block_remove(&mut self, order: usize, block: NonNull<u8>) -> bool
    {
        let block_ptr: NonNull<BlockHeader> = block.cast();
        let mut current: &mut Option<NonNull<BlockHeader>> = &mut self.freelists[order];

        while let Some(ptr) = current
        {
            if *ptr == block_ptr
            {
                *current = unsafe { ptr.read().next };
                return true;
            }

            current = unsafe { &mut ((*(ptr.as_ptr())).next)}
        }

        false
    }

    unsafe fn split_block(&mut self, block: NonNull<u8>, order: usize, target: usize)
    {
        let block_size = self.get_required_block_size(order);

        let mut index = 0;
        while (order >> index) > target
        {
            index += 1;

            let split = unsafe { block.byte_add(block_size >> index) };
            self.block_insert(order - index, split);
        }
    }

    fn find_buddy(&self, order: usize, block: NonNull<u8>) -> Option<NonNull<u8>>
    {
        let relative = unsafe { block.byte_offset_from_unsigned(self.base) };
        let size = self.get_required_block_size(order);

        guard!(size < self.capacity);

        Some(unsafe { self.base.byte_add(relative ^ size) })
    }
}

#[derive(PartialEq, Eq)]
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
