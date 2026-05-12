use std::ptr::NonNull;

use crate::memory::{
    heap::{gc::GarbageCollector, heap::Heap},
    stack::{Stack, entry::StackEntry},
};

pub struct Serial;

impl GarbageCollector for Serial
{
    fn mark(stack: &Stack, _heap: &Heap) -> impl Iterator<Item = NonNull<u8>>
    {
        stack.iter().filter_map(|x| match x
        {
            &StackEntry::Reference(y) => y,
            _ => None,
        })
    }
}
