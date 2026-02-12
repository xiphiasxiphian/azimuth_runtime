use std::ptr::NonNull;

use crate::memory::{heap::heap::Heap, stack::Stack};

pub mod serial;

pub trait GarbageCollector
{
    fn mark(stack: &Stack, heap: &Heap) -> impl Iterator<Item = NonNull<u8>>;
}
