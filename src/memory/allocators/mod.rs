use std::alloc::LayoutError;

pub mod arena;
pub mod general;

const MIN_PAGE_ALIGNMENT: usize = 4096; // Page size

#[derive(Debug)]
pub enum AllocatorError
{
    BadLayout(LayoutError),
    FailedInitialAllocation,
    BadConstraints,
    BadRequest,
}
