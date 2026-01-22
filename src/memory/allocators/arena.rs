use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

use crate::memory::allocators::ALIGNMENT;

pub struct ArenaAllocator
{
    base: NonNull<u8>,
    head_offset: usize,
    capacity: usize,
    layout: Layout,
}

impl Drop for ArenaAllocator
{
    fn drop(&mut self)
    {
        unsafe { dealloc(self.base.as_ptr(), self.layout) };
    }
}

impl ArenaAllocator
{
    pub fn with_capacity(capacity: usize) -> Option<Self>
    {
        let layout = Layout::from_size_align(capacity, super::ALIGNMENT).ok()?;
        let data = unsafe { alloc(layout) };

        Some(Self {
            base: NonNull::new(data)?,
            head_offset: 0,
            capacity,
            layout,
        })
    }

    pub fn raw_alloc(&mut self, size: usize) -> Option<NonNull<u8>>
    {
        (size + self.head_offset <= self.capacity).then(|| {
            let result = unsafe { self.base.byte_add(self.head_offset) };
            self.head_offset += size;

            result
        })
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        let adjusted_size = size_of_val(&value).next_multiple_of(ALIGNMENT);

        (adjusted_size + self.head_offset <= self.capacity).then(|| {
            let ptr: NonNull<T> = unsafe { self.base.byte_add(self.head_offset) }.cast();
            self.head_offset += size_of_val(&value);

            unsafe { ptr.write(value) };
            ptr
        })
    }

    pub fn release_all(&mut self)
    {
        self.head_offset = 0;
    }
}

#[cfg(test)]
mod arena_tests
{
    use super::*;

    struct TestingData
    {
        number: i32,
        character: char,
        boolean: bool,
        text: &'static str,
    }

    #[test]
    fn arena_created()
    {
        let _ = ArenaAllocator::with_capacity(1024).unwrap();
    }

    #[test]
    fn single_allocation()
    {
        let mut arena = ArenaAllocator::with_capacity(1024).unwrap();
        let ptr = arena.alloc(TestingData { number: 1, character: 'a', boolean: false, text: "Hello!" }).unwrap();

        unsafe {
            assert_eq!(ptr.read().boolean, false);
            assert_eq!(ptr.read().number, 1);
            assert_eq!(ptr.read().character, 'a');
            assert_eq!(ptr.read().text, "Hello!");
        }
    }

    #[test]
    fn multi_allocation()
    {
        let mut arena = ArenaAllocator::with_capacity(1024).unwrap();
        let ptrs: Vec<_> = (0..10).map(|x| arena.alloc(x).unwrap()).collect();

        for (i, ptr) in ptrs.iter().enumerate()
        {
            assert_eq!(unsafe { ptr.read() }, i);
        }
    }

    #[test]
    fn multi_sized_allocations()
    {
        let mut arena = ArenaAllocator::with_capacity(1024).unwrap();

        let integer = arena.alloc(5).unwrap();
        let boolean = arena.alloc(true).unwrap();
        let string = arena.alloc("Hello World!").unwrap();
        let character = arena.alloc('b').unwrap();
        let testing_data = arena.alloc(TestingData { number: 1, character: 'a', boolean: false, text: "Hello!" }).unwrap();

        unsafe
        {
            assert_eq!(integer.read(), 5);
            assert_eq!(boolean.read(), true);
            assert_eq!(string.read(), "Hello World!");
            assert_eq!(character.read(), 'b');

            assert_eq!(testing_data.read().boolean, false);
            assert_eq!(testing_data.read().number, 1);
            assert_eq!(testing_data.read().character, 'a');
            assert_eq!(testing_data.read().text, "Hello!");
        }
    }

    #[test]
    fn deallocation()
    {
        let mut arena = ArenaAllocator::with_capacity(1024).unwrap();

        let ptr1 = arena.alloc("Hello!").unwrap();
        assert_eq!(unsafe { ptr1.read() }, "Hello!");

        arena.release_all();

        let ptr2 = arena.alloc("World!").unwrap();
        assert_eq!(unsafe { ptr2.read() }, "World!");

        assert_eq!(ptr1.as_ptr() as usize, ptr2.as_ptr() as usize);
    }
}
