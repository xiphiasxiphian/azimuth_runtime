use std::{alloc::{Layout, alloc, dealloc}, ptr::NonNull};

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
        (size_of_val(&value) + self.head_offset <= self.capacity).then(|| {
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
