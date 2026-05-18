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
    pub vtable_or_type: NonNull<u8>, // Used to find the GC metadata/map of fields
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
    active_teen: usize,
}

impl Heap {
    const MAX_INFANT_SINGLE_ALLOCATION_DIVISOR: usize = 2;

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
            active_teen: 0,
        })
    }

    pub fn raw_alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        // if the object takes up more than 50% allocate to adult
        if layout.size() > (self.infant.capacity() / Self::MAX_INFANT_SINGLE_ALLOCATION_DIVISOR)
        {
            return self.adult.raw_alloc(layout);
        }

        // first attempt
        if let ptr @ Some(_) = self.infant.raw_alloc(layout) { return ptr }

        // infant is full, trigger minor gc
        // self.collect_minor();

        // second attempt
        if let ptr @ Some(_) = self.infant.raw_alloc(layout) { return ptr }

        // if still fails, perform major gc and pray
        // self.collect_major();

        // final attempt, try fallback on adult if required
        self.infant.raw_alloc(layout)
            .or_else(|| self.adult.raw_alloc(layout))
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

    /// Minor GC: Scavenges Infant and Active Teen, promoting survivors.
    pub fn collect_minor(&mut self, roots: &mut Vec<*mut NonNull<u8>>) {
        let from_teen_idx = self.active_teen;
        let to_teen_idx = 1 - self.active_teen;

        // Tenuring threshold: Objects surviving 8 minor GCs move to Adult Gen
        const TENURING_THRESHOLD: usize = 8;

        // We use a queue-based copying mechanism (Cheney's Algorithm)
        // For simplicity in Rust, we'll track objects we need to scan in a worklist
        let mut worklist: Vec<NonNull<u8>> = Vec::new();

        // Helper to check if a pointer resides within a specific memory region
        let in_young_gen = |ptr: NonNull<u8>| {
            // Check if ptr is within infant allocator bounds or active teen bounds
            // (Assuming your allocators expose bounds checking methods)
            true // Stub: replace with actual range checks
        };

        // --- Step 1: Evacuate Roots ---
        for root in roots.iter_mut() {
            unsafe {
                let obj_ptr = **root;
                if in_young_gen(obj_ptr) {
                    **root = self.evacuate(obj_ptr, to_teen_idx, TENURING_THRESHOLD, &mut worklist);
                }
            }
        }

        // --- Step 2: Scan Evacuated Objects (Cheney Tracing) ---
        while let Some(parent_ptr) = worklist.pop() {
            unsafe {
                let header = &*(parent_ptr.as_ptr() as *const ObjectHeader);
                // Get layout/metadata map from the object type system
                let metadata = self.get_metadata(header.vtable_or_type);

                for &offset in metadata.references() {
                    let field_ptr = parent_ptr.as_ptr().add(offset) as *mut NonNull<u8>;
                    let child_ptr = *field_ptr;

                    if in_young_gen(child_ptr) {
                        *field_ptr = self.evacuate(child_ptr, to_teen_idx, TENURING_THRESHOLD, &mut worklist);
                    }
                }
            }
        }

        // --- Step 3: Reset and Swap ---
        // 1. Clear infant (Eden) completely since everything alive moved out.
        self.infant.reset();

        // 2. Clear the old "From" teen space.
        self.teen[from_teen_idx].reset();

        // 3. Swap active survivor spaces.
        self.active_teen = to_teen_idx;
    }

    /// Moves a single object out of danger zone into either "To Space" or "Adult Gen".
    unsafe fn evacuate(
        &mut self,
        obj_ptr: NonNull<u8>,
        to_teen_idx: usize,
        threshold: usize,
        worklist: &mut Vec<NonNull<u8>>
    ) -> NonNull<u8> {
        let header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };

        // If already moved, return its new home immediately
        if header.is_forwarded() {
            return header.forwarding_address();
        }

        let metadata = unsafe { self.get_metadata(header.vtable_or_type) };
        let size = metadata.size();
        let layout = Layout::from_size_align(size, HEAP_ALIGN).unwrap();

        let destination_ptr: NonNull<u8>;

        if header.age() >= threshold {
            // Promote to Adult (Old Generation)
            destination_ptr = self.adult.raw_alloc(layout)
                .expect("Old Gen Out of Memory during promotion!");
        } else {
            // Attempt to copy to "To" Survivor Space
            if let Some(ptr) = self.teen[to_teen_idx].raw_alloc(layout) {
                destination_ptr = ptr;
                // Increment Age inside the new copy's header
                let new_header = &mut *(destination_ptr.as_ptr() as *mut ObjectHeader);
                new_header.mark_word = ((header.age() + 1) << 1) | (header.mark_word & !0x1F);
            } else {
                // Survivor space overflow! Prematurely promote to Adult Gen
                destination_ptr = self.adult.raw_alloc(layout)
                    .expect("Old Gen Out of Memory during premature promotion!");
            }
        }

        // Bitwise copy object data to new destination
        std::ptr::copy_nonoverlapping(obj_ptr.as_ptr(), destination_ptr.as_ptr(), size);

        // Leave behind a forwarding pointer in the old corpse object
        header.set_forwarding_address(destination_ptr);

        // Push to worklist so we can scan this object's children later
        worklist.push(destination_ptr);

        destination_ptr
    }

    unsafe fn get_metadata(&self, _vtable: NonNull<()>) -> &dyn Traceable {
        todo!("Hook this up to your runtime's layout/class dictionary")
    }
}
