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

/// Helper to round up to the nearest multiple of `align`.
/// Note: `align` MUST be a power of two.
const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

struct Ratio(usize, usize);

// JVM NewRatio=2 (Old is 2x the size of Young)
const YOUNG_OLD_RATIO: Ratio = Ratio(1, 2);

// JVM SurvivorRatio=8 (Eden is 8x the size of ONE survivor space).
// For 2 survivor spaces, Eden vs Total Survivor is 8:2, which simplifies to 4:1.
const INFANT_TEEN_RATIO: Ratio = Ratio(4, 1);

impl Ratio {
    pub const fn split(&self, value: usize) -> (usize, usize) {
        let total = self.0 + self.1;
        // Integer arithmetic: multiply first to prevent aggressive truncation
        let first = (value * self.0) / total;
        // Subtract to ensure the two parts sum EXACTLY to `value`
        let second = value - first;
        (first, second)
    }
}

#[derive(Clone, Copy)]
enum PoolType {
    Infant,
    Teen(usize),
    Adult,
}

#[derive(Debug, Clone)]
pub enum HeapError {
    InvalidLayout(LayoutError),
    CannotProvision(AllocatorError),
}

#[repr(C)]
pub struct ObjectHeader {
    pub mark_word: usize, // Used for locking, age tracking, and FORWARDING POINTERS
    pub vtable_or_type: NonNull<()>, // Used to find the GC metadata/map of fields
}

impl ObjectHeader {
    const FORWARDED_BIT: usize = 1 << 0; // High or low bit depending on tagging strategy

    pub fn is_forwarded(&self) -> bool {
        (self.mark_word & Self::FORWARDED_BIT) != 0
    }

    pub fn forwarding_address(&self) -> NonNull<u8> {
        NonNull::new((self.mark_word & !Self::FORWARDED_BIT) as *mut u8)
            .expect("Object Header has become corrupted. This shouldn't be possible")
    }

    pub fn set_forwarding_address(&mut self, addr: NonNull<u8>) {
        self.mark_word = addr.as_ptr() as usize | Self::FORWARDED_BIT;
    }

    pub fn age(&self) -> usize {
        (self.mark_word >> 1) & 0x0F // 4 bits for age tracking (Max 15)
    }
}

pub trait Traceable {
    /// Returns a list of memory offsets inside this object that contain object pointers.
    fn references(&self) -> &[usize];
    /// Returns total size of the allocation including header.
    fn size(&self) -> usize;
}

pub struct Heap {
    base: NonNull<u8>,
    layout: Layout,
    infant: ArenaAllocator,
    teen: [GeneralAllocator<TEEN_ALLOCATOR_DEPTH>; TEEN_COUNT],
    adult: GeneralAllocator<ADULT_ALLOCATOR_DEPTH>,
}

impl Heap {
    pub fn with_capacity(capacity: usize) -> Result<Self, HeapError> {
        // calculate raw splits
        let (young_raw, old_raw) = YOUNG_OLD_RATIO.split(capacity);
        let (infant_raw, teen_total_raw) = INFANT_TEEN_RATIO.split(young_raw);

        // split the teen pool amongst the spaces
        let teen_raw = teen_total_raw / TEEN_COUNT;

        // align sizes to page boundaries
        let infant_capacity = align_up(infant_raw, HEAP_ALIGN);
        let teen_capacity = align_up(teen_raw, HEAP_ALIGN); // per teen space
        let adult_capacity = align_up(old_raw, HEAP_ALIGN);

        // compute final layout
        let total_teen_capacity = teen_capacity * TEEN_COUNT;
        let total_capacity = infant_capacity + total_teen_capacity + adult_capacity;

        let layout = Layout::from_size_align(total_capacity, HEAP_ALIGN)
            .map_err(HeapError::InvalidLayout)?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(HeapError::CannotProvision(AllocatorError::FailedInitialAllocation))?;

        // calculate continuous base offsets
        let infant_base = base;
        let teen_base = unsafe { infant_base.byte_add(infant_capacity) };
        let adult_base = unsafe { teen_base.byte_add(total_teen_capacity) };

        // provision allocators
        let infant = ArenaAllocator::from_existing_allocation(infant_base, infant_capacity);

        let teen = from_fn::<Option<GeneralAllocator<_>>, TEEN_COUNT, _>(|i| {
            GeneralAllocator::from_existing_allocation(
                unsafe { teen_base.byte_add(teen_capacity * i) },
                teen_capacity,
            ).ok()
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

    pub fn raw_alloc(&mut self, layout: Layout) -> Option<NonNull<u8>>
    {
        // allocation first attempt
        let ptr = self.infant.raw_alloc(layout);

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
        self.infant.raw_alloc(layout)
    }

    pub fn alloc<T>(&mut self, value: T) -> Option<NonNull<T>>
    {
        self.raw_alloc(Layout::new::<T>()).map(|x| {
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
