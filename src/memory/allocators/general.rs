// A memory manager manages a block of memory as a heap

use std::{
    alloc::{Layout, alloc, dealloc},
    ptr::NonNull,
};

use crate::{
    common::ScopeMethods as _,
    guard,
    memory::allocators::{AllocatorError, MIN_PAGE_ALIGNMENT},
};

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
        if let Some(layout) = self.layout
        {
            unsafe {
                dealloc(self.base.as_ptr(), layout);
            }
        }
    }
}

impl<const DEPTH: usize> GeneralAllocator<DEPTH>
{
    fn new(base: NonNull<u8>, capacity: usize, layout: Option<Layout>) -> Result<Self, AllocatorError>
    {
        let min_block_size = capacity >> (DEPTH - 1);

        guard!(
            base.as_ptr() as usize & (MIN_PAGE_ALIGNMENT - 1) == 0,
            AllocatorError::BadConstraints
        );
        guard!(capacity >= min_block_size, AllocatorError::BadConstraints);
        guard!(
            min_block_size >= size_of::<BlockHeader>(),
            AllocatorError::BadConstraints
        );
        guard!(capacity.is_power_of_two(), AllocatorError::BadConstraints);

        let freelists = [None; DEPTH].also_mut(|x| x[DEPTH - 1] = Some(base.cast()));

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
        let layout = Layout::from_size_align(capacity, MIN_PAGE_ALIGNMENT).map_err(|x| AllocatorError::BadLayout(x))?;

        let base = NonNull::new(unsafe { alloc(layout) }).ok_or(AllocatorError::FailedInitialAllocation)?;

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
                        self.block_pop(order).inspect(|block| {
                            if order > target
                            {
                                unsafe {
                                    self.split_block(*block, order, target);
                                }
                            }
                        })
                    })
                    .find(Option::is_some)
                    .flatten()
            })
            .unwrap_or(None)
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        self.raw_alloc(size_of_val(&value), align_of_val(&value))
            .map(NonNull::cast)
            .inspect(|x| unsafe { x.write(value) })
    }

    #[expect(clippy::expect_used, reason = "If somehow the align and size, it doesn't make sense")]
    pub fn raw_dealloc(&mut self, ptr: NonNull<u8>, size: usize, align: usize)
    {
        let initial = self
            .get_allocation_order(size, align)
            .expect("Invalid Block Deallocation Request");

        let mut block = ptr;
        for order in initial..DEPTH
        {
            if let Some(buddy) = self.find_buddy(order, block)
                && self.block_remove(order, block)
            {
                block = block.min(buddy);
                continue;
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

        if alignment > size
        {
            size = alignment;
        }
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
                if order == self.freelists.len() - 1
                {
                    self.freelists[order] = None;
                }
                else
                {
                    self.freelists[order] = unsafe { blk.read().next };
                }
            })
            .map(NonNull::cast)
    }

    fn block_insert(&mut self, order: usize, block: NonNull<u8>)
    {
        let new_head = block.cast();
        unsafe {
            new_head.write(BlockHeader {
                next: self.freelists[order],
            });
        };

        self.freelists[order] = Some(new_head);
    }

    fn block_remove(&mut self, order: usize, block: NonNull<u8>) -> bool
    {
        let block_ptr: NonNull<BlockHeader> = block.cast();
        let mut current: &mut Option<NonNull<BlockHeader>> = &mut self.freelists[order];

        while let &mut Some(ptr) = current
        {
            if ptr == block_ptr
            {
                *current = unsafe { ptr.read().next };
                return true;
            }

            current = unsafe { &mut ((*(ptr.as_ptr())).next) }
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

#[derive(Debug, PartialEq, Eq)]
struct BlockHeader
{
    next: Option<NonNull<Self>>,
}

#[cfg(test)]
mod general_allocator_tests
{
    use std::array::from_fn;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestingData
    {
        number: i32,
        character: char,
        boolean: bool,
        text: &'static str,
    }

    impl TestingData
    {
        pub fn new(number: i32, character: char, boolean: bool, text: &'static str) -> Self
        {
            TestingData {
                number,
                character,
                boolean,
                text,
            }
        }
    }

    const CAPACITY: usize = 1 << 16;
    const DEPTH: usize = 8;

    #[test]
    fn create_allocator()
    {
        let _ = GeneralAllocator::<DEPTH>::with_capacity(CAPACITY).unwrap();
    }

    #[test]
    fn create_from_existing()
    {
        let mut base = unsafe { Box::<[u8]>::new_zeroed_slice(CAPACITY).assume_init() };
        let _ = GeneralAllocator::<DEPTH>::from_existing_allocation(NonNull::new(base.as_mut_ptr()).unwrap(), CAPACITY);

        // Maybe test some allocations here
    }

    #[test]
    fn single_allocation()
    {
        let mut allocator = GeneralAllocator::<DEPTH>::with_capacity(CAPACITY).unwrap();
        let ptr = allocator
            .alloc(TestingData {
                number: 1,
                character: 'c',
                boolean: true,
                text: "Azimuth",
            })
            .unwrap();

        let data = unsafe { ptr.read() };

        assert_eq!(data.number, 1);
        assert_eq!(data.character, 'c');
        assert_eq!(data.boolean, true);
        assert_eq!(data.text, "Azimuth");
    }

    #[test]
    fn multiple_allocations()
    {
        let mut allocator = GeneralAllocator::<DEPTH>::with_capacity(CAPACITY).unwrap();
        let ptr1 = allocator
            .alloc(TestingData {
                number: 1,
                character: 'c',
                boolean: true,
                text: "Azimuth",
            })
            .unwrap();
        let ptr2 = allocator.alloc(42).unwrap();
        let ptr3 = allocator.alloc("What is this").unwrap();

        let data1 = unsafe { ptr1.read() };
        let data2 = unsafe { ptr2.read() };
        let data3 = unsafe { ptr3.read() };

        assert_eq!(data1.number, 1);
        assert_eq!(data1.character, 'c');
        assert_eq!(data1.boolean, true);
        assert_eq!(data1.text, "Azimuth");

        assert_eq!(data2, 42);
        assert_eq!(data3, "What is this");
    }

    #[test]
    fn basic_deallocation()
    {
        let mut allocator = GeneralAllocator::<5>::with_capacity(256).unwrap();

        let ptr = allocator.alloc([0_u8; 256]).unwrap();

        let ptr2 = allocator.alloc(42);
        assert_eq!(ptr2, None);

        allocator.dealloc(ptr);

        let ptr2 = allocator.alloc(42).unwrap();
        let data = unsafe { ptr2.read() };

        assert_eq!(data, 42);
    }

    #[test]
    fn complex_management()
    {
        let mut allocator = GeneralAllocator::<DEPTH>::with_capacity(4096).unwrap();

        let testing_data: [TestingData; 20] =
            from_fn(|x| TestingData::new(x as i32, ('a' as u8 + x as u8) as char, (x % 2) != 0, "Azimuth"));

        let mut test_ptrs: [NonNull<TestingData>; 20] = testing_data.clone().map(|x| allocator.alloc(x).unwrap());

        let mut integer_ptrs: [NonNull<usize>; 20] = from_fn(|x| allocator.alloc(x + 100).unwrap());

        // Check validity and deallocate odd entries
        for (i, (correct, test)) in testing_data.iter().zip(test_ptrs).enumerate()
        {
            assert_eq!(correct, unsafe { test.as_ref() });
            if i % 2 == 1
            {
                allocator.dealloc(test);
            }
        }

        for (i, integer) in integer_ptrs.iter().enumerate()
        {
            assert_eq!(i + 100, unsafe { integer.read() });
            if i % 2 == 1
            {
                allocator.dealloc(*integer);
            }
        }

        // Fill entries back in
        for i in (1..20).step_by(2)
        {
            test_ptrs[i] = allocator.alloc(TestingData::new(42, '!', true, "\0")).unwrap();
            integer_ptrs[i] = allocator.alloc(42).unwrap();
        }

        // Check again
        for (i, (correct, test)) in testing_data.iter().zip(test_ptrs).enumerate()
        {
            if i % 2 == 1
            {
                assert_eq!(TestingData::new(42, '!', true, "\0"), unsafe { test.read() });
            }
            else
            {
                assert_eq!(correct, unsafe { test.as_ref() });
            }
        }

        for (i, integer) in integer_ptrs.iter().enumerate()
        {
            if i % 2 == 1
            {
                assert_eq!(42, unsafe { integer.read() })
            }
            else
            {
                assert_eq!(i + 100, unsafe { integer.read() });
            }
        }
    }
}
