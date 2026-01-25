use std::alloc::LayoutError;

pub mod arena;
pub mod general;

const ALIGNMENT: usize = size_of::<usize>();

#[derive(Debug)]
pub enum AllocatorError
{
    InvalidHeapSize,
    BadLayout(LayoutError),
    FailedInitialAllocation,
}
