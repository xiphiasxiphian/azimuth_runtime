use std::{
    alloc::{Layout, LayoutError, alloc},
    array::from_fn,
    ptr::NonNull,
};

use crate::memory::allocators::{AllocatorError, arena::ArenaAllocator, general::GeneralAllocator};

const HEAP_ALIGN: usize = 4096;

const TEEN_COUNT: usize = 2;
const TEEN_ALLOCATOR_DEPTH: usize = 16;

const ADULT_ALLOCATOR_DEPTH: usize = 16;

struct Ratio(u32, u32);
const YOUNG_OLD_RATIO: Ratio = Ratio(1, 2);
const INFANT_TEEN_RATIO: Ratio = Ratio(15, 1);

impl Ratio
{
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "who cares lol"
    )]
    pub const fn split(&self, value: usize) -> (usize, usize)
    {
        let total = self.0 + self.1;

        let first = ((self.0 as f64 / total as f64) * value as f64).round() as usize;
        let second = value - first;

        (first, second)
    }
}

#[derive(Clone, Copy)]
enum PoolType
{
    Infant,
    Teen(usize),
    Adult,
}

#[derive(Debug, Clone)]
pub enum HeapError
{
    InvalidLayout(LayoutError),
    CannotProvision(AllocatorError),
}

pub struct Heap
{
    base: NonNull<u8>,
    layout: Layout,
    infant: ArenaAllocator,
    teen: [GeneralAllocator<TEEN_ALLOCATOR_DEPTH>; TEEN_COUNT],
    adult: GeneralAllocator<ADULT_ALLOCATOR_DEPTH>,
}

impl Heap
{
    pub fn with_capacity(capacity: usize) -> Result<Self, HeapError>
    {
        let (young_init, old_init) = YOUNG_OLD_RATIO.split(capacity);
        let (infant_init, teen_init) = INFANT_TEEN_RATIO.split(young_init);

        let infant_capacity = infant_init.next_power_of_two();
        let teen_capacity = teen_init.next_power_of_two();
        let adult_capacity = old_init.next_power_of_two();

        let total_capacity = infant_capacity + teen_capacity + adult_capacity;

        let layout = Layout::from_size_align(total_capacity, HEAP_ALIGN).map_err(|x| HeapError::InvalidLayout(x))?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(HeapError::CannotProvision(AllocatorError::FailedInitialAllocation))?;
        let infant_base = base;
        let teen_base = unsafe { infant_base.byte_add(infant_capacity) };
        let adult_base = unsafe { teen_base.byte_add(teen_capacity) };

        let infant = ArenaAllocator::from_existing_allocation(infant_base, infant_capacity);
        let teen = from_fn::<Option<GeneralAllocator<_>>, TEEN_COUNT, _>(|x| {
            GeneralAllocator::from_existing_allocation(
                unsafe { teen_base.byte_add((teen_capacity * x) / TEEN_COUNT) },
                teen_capacity / TEEN_COUNT,
            )
            .ok()
        })
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .and_then(|teens| teens.try_into().ok())
        .ok_or(HeapError::CannotProvision(AllocatorError::BadConstraints))?;

        let adult = GeneralAllocator::from_existing_allocation(adult_base, adult_capacity)
            .map_err(HeapError::CannotProvision)?;

        Ok(Self {
            base,
            layout,
            infant,
            teen,
            adult,
        })
    }

    pub fn raw_alloc(&mut self, size: usize, align: usize) -> Option<NonNull<u8>>
    {
        // allocation first attempt
        let ptr = self.infant.raw_alloc(size, align);

        // If the first allocation succeeded, then we can just return it and not
        // have to worry about GC
        if ptr.is_some()
        {
            return ptr;
        }

        // Minor GC
        // TODO

        // Allocation retry.
        // If this allocation fails, its because something as truly gone wrong
        self.infant.raw_alloc(size, align)
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        self.raw_alloc(size_of_val(&value), align_of_val(&value)).map(|x| {
            let new_ptr = x.cast();
            unsafe { new_ptr.write(value) };

            new_ptr
        })
    }

    pub fn dealloc<T>(&mut self, ptr: NonNull<T>)
    {
        match self.get_pool(ptr.cast())
        {
            None | Some(PoolType::Infant) =>
            { /* Do nothing */ }
            Some(PoolType::Teen(index)) => self.teen[index].dealloc(ptr),
            Some(PoolType::Adult) => self.adult.dealloc(ptr),
        }
    }

    fn get_pool(&self, ptr: NonNull<u8>) -> Option<PoolType>
    {
        // This isnt a great implementation but will do for now
        if self.infant.contains(ptr)
        {
            Some(PoolType::Infant)
        }
        else if let Some((index, _)) = self.teen.iter().enumerate().find(|&(_, x)| x.contains(ptr))
        {
            Some(PoolType::Teen(index))
        }
        else if self.adult.contains(ptr)
        {
            Some(PoolType::Adult)
        }
        else
        {
            None
        }
    }
}
