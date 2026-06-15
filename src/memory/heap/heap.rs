use std::{
    alloc::{Layout, LayoutError, alloc},
    array::from_fn,
    collections::BTreeMap,
    mem::transmute,
    ptr::NonNull,
};

use crate::{guard, memory::{
    allocators::{AllocatorError, arena::ArenaAllocator, general::GeneralAllocator},
    datumspace::tables::types::{RuntimeType, RuntimeTypeKind},
    stack::{Stack, entry::StackEntry},
}};

const HEAP_ALIGN: usize = 4096;
const TEEN_COUNT: usize = 2;
const TEEN_ALLOCATOR_DEPTH: usize = 16;
const ADULT_ALLOCATOR_DEPTH: usize = 16;

const CARD_SIZE: usize = 512;
const DIRTY: u8 = 1;
const CLEAN: u8 = 0;

/// A pointer to a heap-allocated object.
pub type ObjRef = NonNull<u8>;

/// A raw pointer to a reference *field* inside an object.
/// The GC reads and overwrites these during evacuation / write barriers.
type FieldPtr = *mut ObjRef;

pub type HeapResult<T> = Result<T, HeapError>;

/// Round `value` up to the nearest multiple of `align` (which must be a power of two).
const fn align_up(value: usize, align: usize) -> usize
{
    (value + align - 1) & !(align - 1)
}

struct Ratio(usize, usize);

const YOUNG_OLD_RATIO: Ratio = Ratio(1, 2);

const INFANT_TEEN_RATIO: Ratio = Ratio(4, 1);

impl Ratio
{
    /// Splits `value` into two parts whose sizes are proportional to `self.0`
    /// and `self.1`.  The two parts sum *exactly* to `value`.
    pub const fn split(&self, value: usize) -> (usize, usize)
    {
        let total = self.0 + self.1;
        let first = (value * self.0) / total;
        let second = value - first; // ensures no rounding loss
        (first, second)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Every heap-allocated object is preceded by this header.
///
/// `mark_word` layout (while the object is *live*):
/// ```text
/// bit 0          : 0 = live, 1 = forwarded
/// bits [4 : 1]   : GC age (0–15 minor collections survived)
/// bits [N : 5]   : reserved / future GC flags
/// ```
/// When the object has been *forwarded* (moved by the GC), `mark_word`
/// holds the new address OR-ed with `FORWARDED_BIT`.  The age bits are
/// meaningless at that point — the object is considered dead.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ObjectHeader
{
    pub mark_word: usize,
    pub vtable_or_type: NonNull<u8>,
    pub size: usize, // includes the size of the header itself
}

impl ObjectHeader
{
    const FORWARDED_BIT: usize = 1;
    const AGE_SHIFT: usize = 1;
    const AGE_BITS: usize = 4;
    const AGE_MASK: usize = (1 << Self::AGE_BITS) - 1; // 0x0F
    const MARK_BIT: usize = 1 << 6;

    pub fn is_marked(&self) -> bool
    {
        (self.mark_word & Self::MARK_BIT) != 0
    }

    pub fn set_mark(&mut self)
    {
        self.mark_word |= Self::MARK_BIT;
    }

    pub fn clear_mark(&mut self)
    {
        self.mark_word &= !Self::MARK_BIT;
    }

    pub fn is_forwarded(&self) -> bool
    {
        (self.mark_word & Self::FORWARDED_BIT) != 0
    }

    /// Returns the address the object was moved to.
    ///
    /// # Panics
    /// Panics if the object is not forwarded, or if the forwarding address is null
    /// (which would indicate heap corruption).
    pub fn forwarding_address(&self) -> ObjRef
    {
        debug_assert!(self.is_forwarded(), "called forwarding_address on a live object");
        NonNull::new((self.mark_word & !Self::FORWARDED_BIT) as *mut u8)
            .expect("ObjectHeader corrupted: null forwarding address")
    }

    pub fn set_forwarding_address(&mut self, addr: ObjRef)
    {
        self.mark_word = addr.as_ptr() as usize | Self::FORWARDED_BIT;
    }

    /// Returns the number of minor GCs this object has survived (0–15).
    pub fn age(&self) -> usize
    {
        (self.mark_word >> Self::AGE_SHIFT) & Self::AGE_MASK
    }

    /// Increments the GC age in-place, saturating at `AGE_MASK`.
    pub fn increment_age(&mut self)
    {
        let new_age = (self.age() + 1).min(Self::AGE_MASK);
        // Clear the old age bits, then write the new value.
        self.mark_word = (self.mark_word & !(Self::AGE_MASK << Self::AGE_SHIFT)) | (new_age << Self::AGE_SHIFT);
    }
}

pub struct Heap
{
    /// Base of the entire contiguous allocation.
    base: NonNull<u8>,
    layout: Layout,

    // Young generation
    infant: ArenaAllocator,
    teen: [GeneralAllocator<TEEN_ALLOCATOR_DEPTH>; TEEN_COUNT],
    active_teen: usize,

    // Old generation
    adult: GeneralAllocator<ADULT_ALLOCATOR_DEPTH>,
    /// Base pointer of the adult region — needed to compute card indices.
    adult_base: NonNull<u8>,

    /// One byte per `CARD_SIZE`-byte region of the adult gen.
    /// Set to `DIRTY` by the write barrier when an old-gen object gains a
    /// pointer into young gen.  Cleared and scanned during minor GC.
    card_table: Vec<u8>,

    /// Mappings of allocated objects. Handy for optimising a couple processes
    teen_live: [BTreeMap<usize, usize>; TEEN_COUNT],
    adult_live: BTreeMap<usize, usize>,
}

impl Heap
{
    /// Objects larger than `infant.capacity() / MAX_INFANT_ALLOC_DIVISOR`
    /// bypass the infant space and go straight to the adult gen.
    const MAX_INFANT_ALLOC_DIVISOR: usize = 2;

    /// Objects that survive this many minor GCs are promoted to the adult gen.
    const ADULT_THRESHOLD: usize = 8;

    pub fn with_capacity(capacity: usize) -> HeapResult<Self>
    {
        // raw region sizes
        let (young_raw, old_raw) = YOUNG_OLD_RATIO.split(capacity);
        let (infant_raw, teen_total_raw) = INFANT_TEEN_RATIO.split(young_raw);
        let teen_raw = teen_total_raw / TEEN_COUNT;

        // infant space uses ArenaAllocator, which safely functions on standard page boundaries.
        let infant_capacity = align_up(infant_raw, HEAP_ALIGN);

        // GeneralAllocator strictly require their total managed space to be a power of two.
        let teen_capacity = teen_raw.next_power_of_two().max(1 << TEEN_ALLOCATOR_DEPTH);
        let adult_capacity = old_raw.next_power_of_two().max(1 << ADULT_ALLOCATOR_DEPTH);

        let total_teen_capacity = teen_capacity * TEEN_COUNT;
        let total_capacity = infant_capacity + total_teen_capacity + adult_capacity;

        // heap allocation
        let layout = Layout::from_size_align(total_capacity, HEAP_ALIGN).map_err(HeapError::InvalidLayout)?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(HeapError::CannotProvision(AllocatorError::FailedInitialAllocation))?;

        // get each regions bases.
        let infant_base = base;
        let teen_base = unsafe { infant_base.byte_add(infant_capacity) };
        let adult_base = unsafe { teen_base.byte_add(total_teen_capacity) };

        let infant = ArenaAllocator::from_existing_allocation(infant_base, infant_capacity);

        let teen = from_fn::<Option<GeneralAllocator<_>>, TEEN_COUNT, _>(|i| {
            GeneralAllocator::from_existing_allocation(unsafe { teen_base.byte_add(teen_capacity * i) }, teen_capacity)
                .ok()
        })
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .and_then(|v| v.try_into().ok())
        .ok_or(HeapError::CannotProvision(AllocatorError::BadConstraints))?;

        let adult = GeneralAllocator::from_existing_allocation(adult_base, adult_capacity)
            .map_err(HeapError::CannotProvision)?;

        let num_cards = adult_capacity / CARD_SIZE;
        let card_table = vec![CLEAN; num_cards];

        Ok(Self {
            base,
            layout,
            infant,
            teen,
            active_teen: 0,
            adult,
            adult_base,
            card_table,
            teen_live: from_fn(|_| BTreeMap::new()),
            adult_live: BTreeMap::new(),
        })
    }

    pub fn alloc_object(&mut self, ty: NonNull<RuntimeType>, stack: &mut Stack) -> Option<ObjRef>
    {
        let runtime_type = unsafe { ty.as_ref() };

        let (instance_size, alignment) = match runtime_type.kind
        {
            RuntimeTypeKind::Struct { instance_size, alignment, .. } => (instance_size, alignment as usize),
            RuntimeTypeKind::Enum { instance_size, alignment, .. } => (instance_size, alignment as usize),
            RuntimeTypeKind::Imported { .. } => todo!("Imported type allocation"),
        };

        // ObjectHeader sits at the very start, so alignment must satisfy it regardless
        // of what the type itself requires.
        let alignment = alignment.max(align_of::<ObjectHeader>());

        let layout = Layout::from_size_align(instance_size, alignment).ok()?;

        let ptr = self.raw_alloc(layout, stack)?;

        unsafe {
            ptr.cast::<ObjectHeader>().write(ObjectHeader {
                mark_word: 0,
                vtable_or_type: ty.cast(),
                size: instance_size,
            });
        }

        Some(ptr)
    }

    /// Allocates an object in the adult generation and records its boundary mapping.
    fn alloc_adult(&mut self, layout: Layout) -> Option<ObjRef>
    {
        let ptr = self.adult.raw_alloc(layout)?;
        let offset = unsafe { ptr.byte_offset_from_unsigned(self.adult_base) };
        self.adult_live.insert(offset, layout.size());

        // // card_offsets update
        // let start_card = offset / CARD_SIZE;
        // let end_card = (offset + layout.size() - 1) / CARD_SIZE;
        // if self.card_offsets[start_card] == usize::MAX
        // {
        //     self.card_offsets[start_card] = offset;
        // }

        // for card_idx in (start_card + 1)..=end_card
        // {
        //     self.card_offsets[card_idx] = offset;
        // }

        Some(ptr)
    }

    fn raw_alloc(&mut self, layout: Layout, stack: &mut Stack) -> Option<ObjRef>
    {
        // large objects skip the infant space entirely.
        if layout.size() > self.infant.capacity() / Self::MAX_INFANT_ALLOC_DIVISOR
        {
            return self.alloc_adult(layout);
        }

        // first try
        if let ptr @ Some(_) = self.infant.raw_alloc(layout)
        {
            return ptr;
        }

        // Perform minor gc
        self.collect_minor(stack);

        // second try
        if let ptr @ Some(_) = self.infant.raw_alloc(layout)
        {
            return ptr;
        }

        // self.collect_major(stack)

        // try and allocate into adult if everything else fails
        self.alloc_adult(layout)
    }

    fn alloc<T>(&mut self, value: T, stack: &mut Stack) -> Option<NonNull<T>>
    {
        self.raw_alloc(Layout::for_value(&value), stack)
            .map(NonNull::cast)
            .inspect(|x| unsafe {
                x.write(value);
            })
    }

    /// Deallocates specific previously allocated block
    pub fn dealloc<T>(&mut self, ptr: NonNull<T>)
    {
        match self.get_pool(ptr.cast())
        {
            None | Some(PoolType::Infant) =>
            {}
            Some(PoolType::Teen(i)) => self.teen[i].dealloc(ptr),
            Some(PoolType::Adult) =>
            {
                let offset = ptr.as_ptr() as usize - self.adult_base.as_ptr() as usize;
                self.adult_live.remove(&offset);
                self.adult.dealloc(ptr);
            }
        }
    }

    /// Must be called on every reference-field write: `obj.field = new_value`.
    pub fn write_barrier(&mut self, field_addr: FieldPtr, new_value: ObjRef)
    {
        // do the actual write
        unsafe {
            *field_addr = new_value;
        }

        if self.is_youth(new_value)
            && let Some(field_ref) = NonNull::new(field_addr.cast::<u8>())
            && let Some(card_idx) = self.card_index_of(field_ref)
        {
            self.card_table[card_idx] = DIRTY;
        }
    }

    /// Scavenges the infant space and the active teen ("from") space.
    pub fn collect_minor(&mut self, stack: &mut Stack)
    {
        let from_teen_idx = self.active_teen;
        let to_teen_idx = 1 - self.active_teen;

        let mut worklist: Vec<ObjRef> = Vec::new();

        for entry in stack.iter_mut()
        {
            if let StackEntry::Reference(Some(obj_ptr)) = entry
                && self.is_youth(*obj_ptr)
            {
                *obj_ptr = unsafe { self.evacuate(*obj_ptr, to_teen_idx, &mut worklist) };
            }
        }

        let dirty_cards: Vec<usize> = self
            .card_table
            .iter_mut()
            .enumerate()
            .filter_map(|(i, status)| {
                if *status == DIRTY
                {
                    *status = CLEAN;
                    Some(i)
                }
                else
                {
                    None
                }
            })
            .collect();

        for card_idx in dirty_cards
        {
            self.scan_card(card_idx, to_teen_idx, &mut worklist);
        }

        while let Some(parent_ptr) = worklist.pop()
        {
            unsafe {
                let header = parent_ptr.cast::<ObjectHeader>().as_ref();
                let (offsets, _) = Self::gc_layout(header.vtable_or_type, parent_ptr);

                let parent_is_adult = matches!(self.get_pool(parent_ptr), Some(PoolType::Adult));

                for offset in offsets
                {
                    let field_ptr: FieldPtr = parent_ptr.as_ptr().add(*offset).cast();
                    let child_ptr = *field_ptr;

                    if self.is_youth(child_ptr)
                    {
                        let new_child = self.evacuate(child_ptr, to_teen_idx, &mut worklist);
                        *field_ptr = new_child;

                        // Keep card table consistent for future GCs.
                        if parent_is_adult
                            && self.is_youth(new_child)
                            && let Some(field_ref) = NonNull::new(field_ptr.cast::<u8>())
                            && let Some(card_idx) = self.card_index_of(field_ref)
                        {
                            self.card_table[card_idx] = DIRTY;
                        }
                    }
                }
            }
        }

        // remove all remaining infants
        self.infant.release_all();

        // purge half teen space
        self.teen[from_teen_idx].release_all();
        self.teen_live[from_teen_idx].clear();

        self.active_teen = to_teen_idx;
    }

    pub fn collect_major(&mut self, stack: &mut Stack)
    {
        // Ensure no young-gen objects remain; this simplifies root enumeration
        // since we only need to find adult-gen roots, not trace through young gen.
        self.collect_minor(stack);

        self.mark_phase(stack);
        self.sweep_phase();
    }

    fn mark_phase(&mut self, stack: &mut Stack)
    {
        let mut worklist: Vec<ObjRef> = Vec::new();

        // Stack roots that point into adult gen
        for entry in stack.iter_mut()
        {
            if let StackEntry::Reference(Some(obj_ptr)) = entry
                && matches!(self.get_pool(*obj_ptr), Some(PoolType::Adult))
            {
                self.mark_object(*obj_ptr, &mut worklist);
            }
        }

        // After collect_minor, any remaining young-gen objects (in to-teen) that
        // point into adult gen are also roots. Scan to-teen for outbound pointers.
        let teen_objects: Vec<ObjRef> = self.teen_live[self.active_teen]
            .iter()
            .map(|(&offset, _)| {
                 unsafe { self.teen[self.active_teen].base().byte_add(offset) }
            })
            .collect();
        for obj_ptr in teen_objects
            {
                let header = unsafe { obj_ptr.cast::<ObjectHeader>().as_ref() };
                let (offsets, _) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };
                let offsets: Vec<usize> = offsets.to_vec();

                for offset in offsets
                {
                    let field_ptr: FieldPtr = unsafe { obj_ptr.as_ptr().add(offset).cast() };
                    let child_ptr = unsafe { *field_ptr };

                    if matches!(self.get_pool(child_ptr), Some(PoolType::Adult))
                    {
                        self.mark_object(child_ptr, &mut worklist);
                    }
                }
            }

        while let Some(obj_ptr) = worklist.pop()
        {
            let header = unsafe { obj_ptr.cast::<ObjectHeader>().as_ref() };
            let (offsets, _) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };

            for &offset in offsets
            {
                let field_ptr: FieldPtr = unsafe { obj_ptr.as_ptr().add(offset).cast() };
                let child_ptr = unsafe { *field_ptr };

                if let Some(PoolType::Adult) = self.get_pool(child_ptr)
                {
                    self.mark_object(child_ptr, &mut worklist);
                }
            }
        }
    }

    fn mark_object(&mut self, ptr: ObjRef, worklist: &mut Vec<ObjRef>)
    {
        let header = unsafe { ptr.cast::<ObjectHeader>().as_mut() };
        if !header.is_marked()
        {
            header.set_mark();
            worklist.push(ptr);
        }
    }

    fn sweep_phase(&mut self)
    {
        let dead_offsets: Vec<usize> = self
            .adult_live
            .iter()
            .filter_map(|(&offset, _)| {
                let obj_ptr = unsafe { self.adult_base.byte_add(offset) };
                let header = unsafe { obj_ptr.cast::<ObjectHeader>().as_mut() };

                if header.is_marked()
                {
                    header.clear_mark(); // reset for next major GC
                    None
                }
                else
                {
                    Some(offset)
                }
            })
            .collect();

        for offset in dead_offsets
        {
            let obj_ptr = unsafe { self.adult_base.byte_add(offset) };
            self.adult_live.remove(&offset);
            self.adult.dealloc(obj_ptr.cast::<u8>());
            // card_offsets entries for this object become stale but are harmless:
            // the BTreeMap range in scan_card won't find a live object there.
        }
    }

    /// Evacuates a single live object out of young gen.
    unsafe fn evacuate(&mut self, obj_ptr: ObjRef, to_teen_idx: usize, worklist: &mut Vec<ObjRef>) -> ObjRef
    {
        let header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };

        if header.is_forwarded()
        {
            return header.forwarding_address();
        }

        let (_, size) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };

        // TODO: work out how errors here will work

        let layout = Layout::from_size_align(size, align_of::<ObjectHeader>())
            .expect("Object metadata returned invalid size or alignment");

        let should_promote = header.age() >= Self::ADULT_THRESHOLD;
        let destination: ObjRef = if should_promote
        {
            self.alloc_adult(layout)
                .expect("OOM in old gen during normal promotion")
        }
        else
        {
            self.teen[to_teen_idx].raw_alloc(layout).inspect(|x| {
                let offset = unsafe { x.byte_offset_from_unsigned(self.teen[to_teen_idx].base()) };
                self.teen_live[to_teen_idx].insert(offset, layout.size());
            }).unwrap_or_else(|| {
                self.alloc_adult(layout)
                    .expect("OOM in old gen during overflow promotion")
            })

        };

        unsafe {
            destination.copy_from_nonoverlapping(obj_ptr, size);
        }

        if !should_promote
        {
            let new_header: &mut ObjectHeader = unsafe { destination.cast().as_mut() };
            new_header.increment_age();
        }

        let old_header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };
        old_header.set_forwarding_address(destination);

        worklist.push(destination);

        destination
    }

    /// Scans one card-sized region of the old gen, evacuating any young-gen
    /// pointers found in reference fields of live objects within that region.
    fn scan_card(&mut self, card_idx: usize, to_teen_idx: usize, worklist: &mut Vec<ObjRef>)
    {
        let card_start_offset = card_idx * CARD_SIZE;
        let card_end_offset = (card_idx + 1) * CARD_SIZE;

        // Range ..card_end_offset gets all objects starting before this card ends.
        // The filter then drops any that finish before this card starts,
        // correctly catching objects that began in a prior card but overlap this one.
        let candidates: Vec<ObjRef> = self
            .adult_live
            .range(..card_end_offset)
            .filter(|(off, sz)| *off + *sz > card_start_offset)
            .map(|(&off, _)| unsafe { NonNull::new_unchecked(self.adult_base.as_ptr().add(off)) })
            .collect();

        for obj_ptr in candidates
        {
            let header = unsafe { &*(obj_ptr.as_ptr() as *const ObjectHeader) };

            if header.is_forwarded()
            {
                continue;
            }

            let (offsets, _) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };
            let offsets: Vec<usize> = offsets.to_vec();

            for offset in offsets
            {
                let field_ptr: FieldPtr = unsafe { obj_ptr.as_ptr().add(offset).cast() };
                let child_ptr = unsafe { *field_ptr };

                if self.is_youth(child_ptr)
                {
                    let new_child = unsafe { self.evacuate(child_ptr, to_teen_idx, worklist) };
                    unsafe {
                        *field_ptr = new_child;
                    }

                    if self.is_youth(new_child)
                        && let Some(field_ref) = NonNull::new(field_ptr.cast::<u8>())
                        && let Some(ci) = self.card_index_of(field_ref)
                    {
                        self.card_table[ci] = DIRTY;
                    }
                }
            }
        }
    }

    fn get_pool(&self, ptr: ObjRef) -> Option<PoolType>
    {
        if self.infant.contains(ptr)
        {
            Some(PoolType::Infant)
        }
        else if let Some((i, _)) = self.teen.iter().enumerate().find(|(_, t)| t.contains(ptr))
        {
            Some(PoolType::Teen(i))
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

    fn is_youth(&self, ptr: ObjRef) -> bool
    {
        matches!(self.get_pool(ptr), Some(PoolType::Infant) | Some(PoolType::Teen(_)))
    }

    /// Returns the card-table index for an adult-gen pointer.
    fn card_index_of(&self, ptr: ObjRef) -> Option<usize>
    {
        guard!(self.adult.contains(ptr));

        let offset = unsafe { ptr.byte_offset_from_unsigned(self.adult_base) };
        Some(offset / CARD_SIZE)
    }

    unsafe fn gc_layout(vtable: NonNull<u8>, obj_base: ObjRef) -> (&'static [usize], usize)
    {
        let ty = unsafe { vtable.cast::<RuntimeType>().as_ref() };
        let page = unsafe { ty.back_pointer.as_ref().get_page() };

        match ty.kind
        {
            RuntimeTypeKind::Struct {
                instance_size,
                gc_offsets_index,
                gc_offsets_count,
                ..
            } =>
            {
                let start = gc_offsets_index as usize;
                let offsets = &page.gc_offsets[start..start + gc_offsets_count as usize];
                (unsafe { transmute(offsets) }, instance_size)
            }
            RuntimeTypeKind::Enum {
                instance_size,
                variants_index,
                variants_count,
                ..
            } =>
            {
                let tag = unsafe { obj_base.byte_add(size_of::<ObjectHeader>()).cast::<u32>().read() };
                let start = variants_index as usize;
                let variants = &page.enum_variants[start..start + variants_count as usize];
                let variant = variants
                    .iter()
                    .find(|v| v.tag == tag)
                    .expect("GC: unknown enum tag — heap corrupted");
                let offsets = page
                    .get_variant_gc_offsets(variant)
                    .expect("GC: invalid gc_offsets range in variant");
                (unsafe { transmute(offsets) }, instance_size)
            }
            RuntimeTypeKind::Imported { .. } =>
            {
                todo!("Whats the plan here")
            }
        }
    }
}

#[cfg(test)]
mod tests
{
    use std::{alloc::Layout, ptr::NonNull};

    use super::*;

    // =========================================================================
    // Helpers
    // =========================================================================

    const TEST_HEAP_SIZE: usize = 64 * 1024 * 1024;
    const SMALL_HEAP_SIZE: usize = 4 * 1024 * 1024;

    const STACK_SIZE: usize = 1 << 16;

    fn make_heap() -> Heap
    {
        Heap::with_capacity(TEST_HEAP_SIZE).unwrap()
    }

    fn make_stack() -> Stack
    {
        Stack::new(STACK_SIZE)
    }

    fn fresh_header() -> ObjectHeader
    {
        ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 64,
        }
    }

    /// Allocate directly into the adult gen and return the pointer.
    /// Does not go through raw_alloc, so won't trigger GC.
    fn alloc_adult_raw(heap: &mut Heap, size: usize) -> ObjRef
    {
        let layout = Layout::from_size_align(size, align_of::<ObjectHeader>()).unwrap();
        heap.alloc_adult(layout).expect("adult alloc failed in test")
    }

    /// Allocate directly into the infant space.
    fn alloc_infant_raw(heap: &mut Heap, size: usize) -> ObjRef
    {
        let layout = Layout::from_size_align(size, align_of::<ObjectHeader>()).unwrap();
        heap.infant.raw_alloc(layout).expect("infant alloc failed in test")
    }

    // =========================================================================
    // align_up
    // =========================================================================

    mod align_up_tests
    {
        use super::*;

        #[test]
        fn zero_with_any_alignment_is_zero()
        {
            assert_eq!(align_up(0, 1), 0);
            assert_eq!(align_up(0, 8), 0);
            assert_eq!(align_up(0, 4096), 0);
        }

        #[test]
        fn already_aligned_unchanged()
        {
            assert_eq!(align_up(8, 8), 8);
            assert_eq!(align_up(16, 16), 16);
            assert_eq!(align_up(512, 512), 512);
            assert_eq!(align_up(4096, 4096), 4096);
            assert_eq!(align_up(8192, 4096), 8192);
        }

        #[test]
        fn one_below_alignment_rounds_up()
        {
            assert_eq!(align_up(7, 8), 8);
            assert_eq!(align_up(15, 16), 16);
            assert_eq!(align_up(511, 512), 512);
            assert_eq!(align_up(4095, 4096), 4096);
        }

        #[test]
        fn one_above_alignment_rounds_to_next()
        {
            assert_eq!(align_up(9, 8), 16);
            assert_eq!(align_up(17, 16), 32);
            assert_eq!(align_up(513, 512), 1024);
            assert_eq!(align_up(4097, 4096), 8192);
        }

        #[test]
        fn value_one_with_various_alignments()
        {
            assert_eq!(align_up(1, 1), 1);
            assert_eq!(align_up(1, 2), 2);
            assert_eq!(align_up(1, 8), 8);
            assert_eq!(align_up(1, 4096), 4096);
        }

        #[test]
        fn non_power_of_two_values()
        {
            assert_eq!(align_up(100, 64), 128);
            assert_eq!(align_up(200, 64), 256);
            assert_eq!(align_up(123456, 512), 123904);
            assert_eq!(align_up(300, 256), 512);
        }

        #[test]
        fn alignment_of_one_is_identity()
        {
            assert_eq!(align_up(0, 1), 0);
            assert_eq!(align_up(1, 1), 1);
            assert_eq!(align_up(99999, 1), 99999);
        }

        #[test]
        fn large_values()
        {
            assert_eq!(align_up(1_000_000, 4096), 1_003_520);
            let base: usize = 1 << 30;
            assert_eq!(align_up(base, 4096), base); // already aligned
            assert_eq!(align_up(base + 1, 4096), base + 4096);
        }
    }

    // =========================================================================
    // Ratio::split
    // =========================================================================

    mod ratio_tests
    {
        use super::*;

        #[test]
        fn exact_proportions()
        {
            assert_eq!(Ratio(1, 2).split(300), (100, 200));
            assert_eq!(Ratio(4, 1).split(100), (80, 20));
            assert_eq!(Ratio(1, 1).split(100), (50, 50));
            assert_eq!(Ratio(3, 1).split(100), (75, 25));
        }

        #[test]
        fn sum_always_equals_input()
        {
            let cases = [0usize, 1, 7, 11, 100, 1024, 65536, TEST_HEAP_SIZE];
            let ratios = [Ratio(1, 2), Ratio(4, 1), Ratio(3, 7), Ratio(99, 1)];
            for total in cases
            {
                for ratio in &ratios
                {
                    let (a, b) = ratio.split(total);
                    assert_eq!(a + b, total, "Ratio({},{}).split({}) should sum to input", ratio.0, ratio.1, total);
                }
            }
        }

        #[test]
        fn zero_total_gives_zero_zero()
        {
            assert_eq!(Ratio(1, 2).split(0), (0, 0));
            assert_eq!(Ratio(99, 1).split(0), (0, 0));
            assert_eq!(Ratio(1, 1).split(0), (0, 0));
        }

        #[test]
        fn imperfect_rounding_floors_first_part()
        {
            // 11 * 4/5 = 8.8 → floors to 8
            assert_eq!(Ratio(4, 1).split(11), (8, 3));
            // 1 * 1/3 = 0.33 → floors to 0
            assert_eq!(Ratio(1, 2).split(1), (0, 1));
            // 10 * 1/3 = 3.33 → floors to 3
            assert_eq!(Ratio(1, 2).split(10), (3, 7));
        }

        #[test]
        fn proportions_are_approximately_correct()
        {
            let (a, b) = Ratio(1, 2).split(TEST_HEAP_SIZE);
            // a should be roughly 1/3 of total
            let expected_a = TEST_HEAP_SIZE / 3;
            let tolerance = TEST_HEAP_SIZE / 100; // 1% tolerance
            assert!((a as isize - expected_a as isize).unsigned_abs() < tolerance);
            let _ = b;
        }

        #[test]
        fn ratio_99_to_1_heavily_skewed()
        {
            let (a, b) = Ratio(99, 1).split(1000);
            assert_eq!(a, 990);
            assert_eq!(b, 10);
        }

        #[test]
        fn ratio_value_of_one()
        {
            // Can't truly split 1 byte 1:2 — floor wins
            let (a, b) = Ratio(1, 2).split(1);
            assert_eq!(a + b, 1);
            assert!(a <= 1 && b <= 1);
        }
    }

    // =========================================================================
    // ObjectHeader — mark bit
    // =========================================================================

    mod header_mark_tests
    {
        use super::*;

        #[test]
        fn initial_not_marked()
        {
            assert!(!fresh_header().is_marked());
        }

        #[test]
        fn set_mark_marks()
        {
            let mut h = fresh_header();
            h.set_mark();
            assert!(h.is_marked());
        }

        #[test]
        fn clear_mark_unmarks()
        {
            let mut h = fresh_header();
            h.set_mark();
            h.clear_mark();
            assert!(!h.is_marked());
        }

        #[test]
        fn set_mark_idempotent()
        {
            let mut h = fresh_header();
            h.set_mark();
            h.set_mark();
            assert!(h.is_marked());
        }

        #[test]
        fn clear_mark_on_unmarked_is_idempotent()
        {
            let mut h = fresh_header();
            h.clear_mark(); // no-op
            assert!(!h.is_marked());
        }

        #[test]
        fn multiple_mark_clear_cycles()
        {
            let mut h = fresh_header();
            for _ in 0..20
            {
                h.set_mark();
                assert!(h.is_marked());
                h.clear_mark();
                assert!(!h.is_marked());
            }
        }

        #[test]
        fn mark_does_not_set_forwarded_bit()
        {
            let mut h = fresh_header();
            h.set_mark();
            assert!(!h.is_forwarded());
        }

        #[test]
        fn clear_mark_does_not_affect_forwarded()
        {
            // Forwarded bit lives at bit 0, mark at bit 6 — independent
            let mut h = fresh_header();
            h.set_mark();
            // Manually set the forwarded bit to verify clear_mark doesn't touch it
            // (note: in practice, forwarding overwrites mark_word entirely, but
            // we can test bit independence directly)
            h.mark_word |= ObjectHeader::FORWARDED_BIT;
            h.clear_mark();
            assert!(!h.is_marked());
            // forwarded bit should still be set
            assert!((h.mark_word & ObjectHeader::FORWARDED_BIT) != 0);
        }

        #[test]
        fn mark_bit_independent_of_age_bits()
        {
            let mut h = fresh_header();
            // Set age to 7
            for _ in 0..7
            {
                h.increment_age();
            }
            assert_eq!(h.age(), 7);

            h.set_mark();
            assert_eq!(h.age(), 7); // age unchanged

            h.clear_mark();
            assert_eq!(h.age(), 7); // age still unchanged
            assert!(!h.is_marked());
        }

        #[test]
        fn age_increment_does_not_affect_mark()
        {
            let mut h = fresh_header();
            h.set_mark();

            for _ in 0..10
            {
                h.increment_age();
            }

            assert!(h.is_marked(), "mark bit should survive age increments");
        }
    }

    // =========================================================================
    // ObjectHeader — age
    // =========================================================================

    mod header_age_tests
    {
        use super::*;

        #[test]
        fn initial_age_is_zero()
        {
            assert_eq!(fresh_header().age(), 0);
        }

        #[test]
        fn age_increments_one_at_a_time()
        {
            let mut h = fresh_header();
            for expected in 1..=15
            {
                h.increment_age();
                assert_eq!(h.age(), expected);
            }
        }

        #[test]
        fn age_saturates_at_15()
        {
            let mut h = fresh_header();
            for _ in 0..30
            {
                h.increment_age();
            }
            assert_eq!(h.age(), 15);
        }

        #[test]
        fn age_saturation_at_exactly_15_does_not_overflow()
        {
            let mut h = fresh_header();
            for _ in 0..15
            {
                h.increment_age();
            }
            assert_eq!(h.age(), 15);
            h.increment_age(); // one more — should stay at 15
            assert_eq!(h.age(), 15);
        }

        #[test]
        fn age_increment_never_sets_forwarded_bit()
        {
            let mut h = fresh_header();
            for _ in 0..20
            {
                h.increment_age();
                assert!(!h.is_forwarded(), "age increment set forwarded bit at age {}", h.age());
            }
        }

        #[test]
        fn age_not_affected_by_mark_operations()
        {
            let mut h = fresh_header();
            h.increment_age();
            h.increment_age();
            h.increment_age();
            let age_before = h.age();

            h.set_mark();
            assert_eq!(h.age(), age_before);

            h.clear_mark();
            assert_eq!(h.age(), age_before);
        }

        #[test]
        fn high_bits_preserved_through_age_increment()
        {
            // Bits [7:] set, age = 0
            let high_bits_mask: usize = !0x7F; // bits 7 and above
            let mut h = ObjectHeader {
                mark_word: high_bits_mask,
                vtable_or_type: NonNull::dangling(),
                size: 64,
            };
            assert_eq!(h.age(), 0);
            h.increment_age();
            assert_eq!(h.age(), 1);
            // High bits above bit 6 should be preserved
            assert_eq!(h.mark_word & high_bits_mask, high_bits_mask);
        }

        #[test]
        fn age_bits_exactly_in_bits_1_to_4()
        {
            // Verify bits [4:1] are the age bits by checking no other bit is set after increment from 0
            let mut h = fresh_header();
            h.increment_age(); // age = 1 → bit 1 set
            let age_bits_mask: usize = 0x1E; // bits [4:1]
            assert_eq!(h.mark_word & !age_bits_mask, 0, "only age bits should be set after first increment");
        }

        #[test]
        fn all_15_age_values_roundtrip()
        {
            let mut h = fresh_header();
            for expected_age in 0..=15usize
            {
                assert_eq!(h.age(), expected_age);
                if expected_age < 15
                {
                    h.increment_age();
                }
            }
        }
    }

    // =========================================================================
    // ObjectHeader — forwarding
    // =========================================================================

    mod header_forwarding_tests
    {
        use super::*;

        #[test]
        fn initial_not_forwarded()
        {
            assert!(!fresh_header().is_forwarded());
        }

        #[test]
        fn set_and_recover_forwarding_address()
        {
            let mut h = fresh_header();
            let target = NonNull::new(0xABCD_1200 as *mut u8).unwrap();
            h.set_forwarding_address(target);
            assert!(h.is_forwarded());
            assert_eq!(h.forwarding_address(), target);
        }

        #[test]
        fn forwarding_with_various_aligned_addresses()
        {
            // Addresses must have bit 0 clear for the FORWARDED_BIT trick to work
            let addrs: &[usize] = &[0x1000, 0x2000, 0x10000, 0x100000, 0x7FFF_F000, 0xABCD_1200];
            for &raw in addrs
            {
                let mut h = fresh_header();
                let addr = NonNull::new(raw as *mut u8).unwrap();
                h.set_forwarding_address(addr);
                assert!(h.is_forwarded(), "addr 0x{:X} should be forwarded", raw);
                assert_eq!(
                    h.forwarding_address().as_ptr() as usize,
                    raw,
                    "recovered address should match 0x{:X}",
                    raw
                );
            }
        }

        #[test]
        #[should_panic(expected = "called forwarding_address on a live object")]
        fn forwarding_address_panics_on_live_object()
        {
            let h = fresh_header();
            let _ = h.forwarding_address();
        }

        #[test]
        fn set_forwarding_address_overwrites_mark_word_entirely()
        {
            let mut h = fresh_header();
            h.set_mark();
            h.increment_age();
            h.increment_age();
            // Now overwrite via forwarding
            let addr = NonNull::new(0x8000 as *mut u8).unwrap();
            h.set_forwarding_address(addr);
            assert!(h.is_forwarded());
            assert_eq!(h.forwarding_address(), addr);
        }

        #[test]
        fn is_forwarded_false_after_fresh_mark()
        {
            let mut h = fresh_header();
            h.set_mark();
            assert!(!h.is_forwarded());
        }

        #[test]
        fn forwarding_replaces_previous_forwarding()
        {
            let mut h = fresh_header();
            let addr1 = NonNull::new(0x1000 as *mut u8).unwrap();
            let addr2 = NonNull::new(0x2000 as *mut u8).unwrap();
            h.set_forwarding_address(addr1);
            h.set_forwarding_address(addr2);
            assert_eq!(h.forwarding_address(), addr2);
        }
    }

    // =========================================================================
    // Heap initialisation
    // =========================================================================

    mod heap_init_tests
    {
        use super::*;

        #[test]
        fn basic_construction_succeeds()
        {
            let _ = make_heap();
        }

        #[test]
        fn small_heap_construction_succeeds()
        {
            let _ = Heap::with_capacity(SMALL_HEAP_SIZE).unwrap();
        }

        #[test]
        fn active_teen_starts_at_zero()
        {
            assert_eq!(make_heap().active_teen, 0);
        }

        #[test]
        fn card_table_all_clean_on_init()
        {
            let heap = make_heap();
            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn card_table_not_empty()
        {
            assert!(!make_heap().card_table.is_empty());
        }

        #[test]
        fn adult_live_empty_on_init()
        {
            assert!(make_heap().adult_live.is_empty());
        }

        #[test]
        fn teen_live_all_empty_on_init()
        {
            let heap = make_heap();
            for map in &heap.teen_live
            {
                assert!(map.is_empty());
            }
        }

        #[test]
        fn teen_live_has_correct_count()
        {
            assert_eq!(make_heap().teen_live.len(), TEEN_COUNT);
        }

        #[test]
        fn card_table_size_corresponds_to_adult_region()
        {
            // card_table.len() * CARD_SIZE must equal the adult capacity.
            // We can't read adult_capacity directly, but we can verify the table is
            // a power-of-two multiple of CARD_SIZE entries.
            let heap = make_heap();
            let num_cards = heap.card_table.len();
            assert!(num_cards > 0);
            // Each card covers CARD_SIZE bytes; total adult bytes must be a power of two.
            assert!((num_cards * CARD_SIZE).is_power_of_two());
        }

        #[test]
        fn two_heaps_are_independent()
        {
            let mut heap1 = make_heap();
            let mut heap2 = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let p1 = heap1.alloc_adult(layout).unwrap();
            let p2 = heap2.alloc_adult(layout).unwrap();
            // Different heaps, different base allocations — pointers must differ
            assert_ne!(p1, p2);
            assert_eq!(heap1.adult_live.len(), 1);
            assert_eq!(heap2.adult_live.len(), 1);
        }
    }

    // =========================================================================
    // get_pool and is_youth
    // =========================================================================

    mod pool_classification_tests
    {
        use super::*;

        #[test]
        fn external_pointer_is_none()
        {
            let heap = make_heap();
            let external = NonNull::new(0x1000 as *mut u8).unwrap();
            assert_eq!(heap.get_pool(external), None);
        }

        #[test]
        fn another_external_pointer_is_none()
        {
            let heap = make_heap();
            let external = NonNull::new(0x5000 as *mut u8).unwrap();
            assert_eq!(heap.get_pool(external), None);
        }

        #[test]
        fn infant_allocation_classified_as_infant()
        {
            let mut heap = make_heap();
            let ptr = alloc_infant_raw(&mut heap, 64);
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Infant));
        }

        #[test]
        fn multiple_infant_allocations_classified_as_infant()
        {
            let mut heap = make_heap();
            for _ in 0..5
            {
                let ptr = alloc_infant_raw(&mut heap, 64);
                assert_eq!(heap.get_pool(ptr), Some(PoolType::Infant));
            }
        }

        #[test]
        fn adult_allocation_classified_as_adult()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Adult));
        }

        #[test]
        fn multiple_adult_allocations_classified_as_adult()
        {
            let mut heap = make_heap();
            for _ in 0..5
            {
                let ptr = alloc_adult_raw(&mut heap, 64);
                assert_eq!(heap.get_pool(ptr), Some(PoolType::Adult));
            }
        }

        #[test]
        fn teen0_allocation_classified_as_teen_0()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.teen[0].raw_alloc(layout).unwrap();
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Teen(0)));
        }

        #[test]
        fn teen1_allocation_classified_as_teen_1()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.teen[1].raw_alloc(layout).unwrap();
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Teen(1)));
        }

        #[test]
        fn is_youth_infant_is_true()
        {
            let mut heap = make_heap();
            let ptr = alloc_infant_raw(&mut heap, 64);
            assert!(heap.is_youth(ptr));
        }

        #[test]
        fn is_youth_teen0_is_true()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.teen[0].raw_alloc(layout).unwrap();
            assert!(heap.is_youth(ptr));
        }

        #[test]
        fn is_youth_teen1_is_true()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.teen[1].raw_alloc(layout).unwrap();
            assert!(heap.is_youth(ptr));
        }

        #[test]
        fn is_youth_adult_is_false()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            assert!(!heap.is_youth(ptr));
        }

        #[test]
        fn is_youth_external_is_false()
        {
            let heap = make_heap();
            let external = NonNull::new(0x1000 as *mut u8).unwrap();
            assert!(!heap.is_youth(external));
        }
    }

    // =========================================================================
    // card_index_of
    // =========================================================================

    mod card_index_tests
    {
        use super::*;

        #[test]
        fn external_pointer_returns_none()
        {
            let heap = make_heap();
            assert_eq!(heap.card_index_of(NonNull::new(0x1000 as *mut u8).unwrap()), None);
        }

        #[test]
        fn infant_pointer_returns_none()
        {
            let mut heap = make_heap();
            let ptr = alloc_infant_raw(&mut heap, 64);
            assert_eq!(heap.card_index_of(ptr), None);
        }

        #[test]
        fn teen_pointer_returns_none()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.teen[0].raw_alloc(layout).unwrap();
            assert_eq!(heap.card_index_of(ptr), None);
        }

        #[test]
        fn adult_pointer_returns_some()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            assert!(heap.card_index_of(ptr).is_some());
        }

        #[test]
        fn adult_index_within_card_table_bounds()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            let idx = heap.card_index_of(ptr).unwrap();
            assert!(idx < heap.card_table.len());
        }

        #[test]
        fn card_index_increases_with_offset()
        {
            // Allocate one object per card and verify indices ascend
            let mut heap = make_heap();
            let layout = Layout::from_size_align(CARD_SIZE, 8).unwrap();
            let ptr1 = heap.alloc_adult(layout).unwrap();
            let ptr2 = heap.alloc_adult(layout).unwrap();
            let idx1 = heap.card_index_of(ptr1).unwrap();
            let idx2 = heap.card_index_of(ptr2).unwrap();
            // Second allocation is at a higher address
            assert!(idx2 >= idx1);
        }

        #[test]
        fn two_ptrs_in_same_card_give_same_index()
        {
            let mut heap = make_heap();
            // Two small allocations should land in the same card if CARD_SIZE is large enough
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr1 = heap.alloc_adult(layout).unwrap();
            let ptr2 = heap.alloc_adult(layout).unwrap();
            let idx1 = heap.card_index_of(ptr1).unwrap();
            let idx2 = heap.card_index_of(ptr2).unwrap();
            // Both 64-byte allocations should sit in the same 512-byte card
            assert_eq!(idx1, idx2);
        }

        #[test]
        fn adult_base_pointer_is_card_zero()
        {
            let heap = make_heap();
            // adult_base itself should be in card 0
            assert_eq!(heap.card_index_of(heap.adult_base), Some(0));
        }

        #[test]
        fn offset_exactly_one_card_size_is_card_one()
        {
            let heap = make_heap();
            let ptr = unsafe { heap.adult_base.byte_add(CARD_SIZE) };
            assert_eq!(heap.card_index_of(ptr), Some(1));
        }

        #[test]
        fn offset_one_byte_before_card_boundary_is_previous_card()
        {
            let heap = make_heap();
            let ptr = unsafe { heap.adult_base.byte_add(CARD_SIZE - 1) };
            assert_eq!(heap.card_index_of(ptr), Some(0));
        }
    }

    // =========================================================================
    // adult_live tracking
    // =========================================================================

    mod adult_live_tests
    {
        use super::*;

        #[test]
        fn empty_initially()
        {
            assert!(make_heap().adult_live.is_empty());
        }

        #[test]
        fn single_allocation_appears()
        {
            let mut heap = make_heap();
            alloc_adult_raw(&mut heap, 64);
            assert_eq!(heap.adult_live.len(), 1);
        }

        #[test]
        fn multiple_allocations_all_recorded()
        {
            let mut heap = make_heap();
            for _ in 0..10
            {
                alloc_adult_raw(&mut heap, 64);
            }
            assert_eq!(heap.adult_live.len(), 10);
        }

        #[test]
        fn dealloc_removes_entry()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            assert_eq!(heap.adult_live.len(), 1);
            heap.dealloc::<u8>(ptr);
            assert_eq!(heap.adult_live.len(), 0);
        }

        #[test]
        fn dealloc_removes_correct_entry()
        {
            let mut heap = make_heap();
            let ptr1 = alloc_adult_raw(&mut heap, 64);
            let ptr2 = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr1);
            assert_eq!(heap.adult_live.len(), 1);
            let offset2 = unsafe { ptr2.byte_offset_from_unsigned(heap.adult_base) };
            assert!(heap.adult_live.contains_key(&offset2));
        }

        #[test]
        fn correct_offset_recorded()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            let offset = unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) };
            assert!(heap.adult_live.contains_key(&offset));
        }

        #[test]
        fn correct_size_recorded()
        {
            let mut heap = make_heap();
            let size = 128usize;
            let layout = Layout::from_size_align(size, 8).unwrap();
            let ptr = heap.alloc_adult(layout).unwrap();
            let offset = unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) };
            let &recorded_size = heap.adult_live.get(&offset).unwrap();
            assert_eq!(recorded_size, size);
        }

        #[test]
        fn all_offsets_are_distinct()
        {
            let mut heap = make_heap();
            let mut offsets = Vec::new();
            for _ in 0..5
            {
                let ptr = alloc_adult_raw(&mut heap, 64);
                let offset = unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) };
                offsets.push(offset);
            }
            offsets.sort();
            offsets.dedup();
            assert_eq!(offsets.len(), 5, "all offsets should be distinct");
        }

        #[test]
        fn dealloc_then_realloc_restores_count()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr);
            alloc_adult_raw(&mut heap, 64);
            assert_eq!(heap.adult_live.len(), 1);
        }

        #[test]
        fn offsets_are_in_ascending_order_in_btreemap()
        {
            let mut heap = make_heap();
            for _ in 0..5
            {
                alloc_adult_raw(&mut heap, 64);
            }
            let offsets: Vec<usize> = heap.adult_live.keys().copied().collect();
            let mut sorted = offsets.clone();
            sorted.sort();
            assert_eq!(offsets, sorted, "BTreeMap should yield ascending offsets");
        }
    }

    // =========================================================================
    // write_barrier
    // =========================================================================

    mod write_barrier_tests
    {
        use super::*;

        #[test]
        fn adult_field_to_youth_dirties_card()
        {
            let mut heap = make_heap();
            let adult = alloc_adult_raw(&mut heap, 64);
            let youth = alloc_infant_raw(&mut heap, 64);

            let field: FieldPtr = adult.as_ptr().cast();
            heap.write_barrier(field, youth);

            let card = heap.card_index_of(adult).unwrap();
            assert_eq!(heap.card_table[card], DIRTY);
        }

        #[test]
        fn adult_field_to_adult_does_not_dirty()
        {
            let mut heap = make_heap();
            let adult1 = alloc_adult_raw(&mut heap, 64);
            let adult2 = alloc_adult_raw(&mut heap, 64);

            let field: FieldPtr = adult1.as_ptr().cast();
            heap.write_barrier(field, adult2);

            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn infant_field_to_youth_does_not_dirty_card()
        {
            // Infant fields are not tracked by the card table
            let mut heap = make_heap();
            let infant1 = alloc_infant_raw(&mut heap, 64);
            let infant2 = alloc_infant_raw(&mut heap, 64);

            let field: FieldPtr = infant1.as_ptr().cast();
            heap.write_barrier(field, infant2);

            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn write_barrier_actually_performs_write()
        {
            let mut heap = make_heap();
            let adult = alloc_adult_raw(&mut heap, 64);
            let youth = alloc_infant_raw(&mut heap, 64);

            let field: FieldPtr = adult.as_ptr().cast();
            heap.write_barrier(field, youth);

            assert_eq!(unsafe { *field }, youth);
        }

        #[test]
        fn multiple_writes_to_same_card_stays_dirty()
        {
            let mut heap = make_heap();
            let adult = alloc_adult_raw(&mut heap, 64);
            let y1 = alloc_infant_raw(&mut heap, 64);
            let y2 = alloc_infant_raw(&mut heap, 64);

            let field: FieldPtr = adult.as_ptr().cast();
            heap.write_barrier(field, y1);
            heap.write_barrier(field, y2);

            let card = heap.card_index_of(adult).unwrap();
            assert_eq!(heap.card_table[card], DIRTY);
        }

        #[test]
        fn write_to_adult_then_adult_leaves_card_clean()
        {
            let mut heap = make_heap();
            let adult1 = alloc_adult_raw(&mut heap, 64);
            let adult2 = alloc_adult_raw(&mut heap, 64);
            let adult3 = alloc_adult_raw(&mut heap, 64);

            let field: FieldPtr = adult1.as_ptr().cast();
            heap.write_barrier(field, adult2);
            heap.write_barrier(field, adult3);

            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn write_to_teen_field_pointing_to_youth_does_not_dirty()
        {
            let mut heap = make_heap();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let teen_ptr = heap.teen[0].raw_alloc(layout).unwrap();
            let youth = alloc_infant_raw(&mut heap, 64);

            let field: FieldPtr = teen_ptr.as_ptr().cast();
            heap.write_barrier(field, youth);

            // Teen is not in the adult card table
            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }
    }

    // =========================================================================
    // scan_card range query (via adult_live)
    // =========================================================================

    mod scan_card_range_tests
    {
        use super::*;

        fn candidates_for_card(heap: &Heap, card_idx: usize) -> Vec<usize>
        {
            let card_start = card_idx * CARD_SIZE;
            let card_end = (card_idx + 1) * CARD_SIZE;
            heap.adult_live
                .range(..card_end)
                .filter(|(off, sz)| *off + *sz > card_start)
                .map(|(&off, _)| off)
                .collect()
        }

        #[test]
        fn object_within_card_is_found()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            let offset = unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) };
            let card_idx = offset / CARD_SIZE;
            let candidates = candidates_for_card(&heap, card_idx);
            assert!(candidates.contains(&offset));
        }

        #[test]
        fn object_spanning_into_next_card_found_in_both()
        {
            let mut heap = make_heap();
            // Allocate object large enough to cross a card boundary
            let layout = Layout::from_size_align(CARD_SIZE * 2, 8).unwrap();
            let ptr = heap.alloc_adult(layout).unwrap();
            let offset = unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) };
            let card_idx = offset / CARD_SIZE;

            // Should appear in its own card
            assert!(candidates_for_card(&heap, card_idx).contains(&offset));
            // And in the next card it overlaps
            assert!(candidates_for_card(&heap, card_idx + 1).contains(&offset));
        }

        #[test]
        fn empty_heap_returns_no_candidates()
        {
            let heap = make_heap();
            for card_idx in 0..5
            {
                assert!(candidates_for_card(&heap, card_idx).is_empty());
            }
        }

        #[test]
        fn object_does_not_appear_in_distant_card()
        {
            let mut heap = make_heap();
            alloc_adult_raw(&mut heap, 64); // allocates at/near offset 0
            // Card 50 should be empty
            assert!(candidates_for_card(&heap, 50).is_empty());
        }

        #[test]
        fn multiple_objects_in_same_card_all_found()
        {
            let mut heap = make_heap();
            // Several small allocations — all land in card 0
            let mut offsets = Vec::new();
            for _ in 0..4
            {
                let ptr = alloc_adult_raw(&mut heap, 64);
                offsets.push(unsafe { ptr.byte_offset_from_unsigned(heap.adult_base) });
            }
            let candidates = candidates_for_card(&heap, 0);
            for offset in offsets
            {
                assert!(candidates.contains(&offset), "offset {} should be in card 0", offset);
            }
        }

        #[test]
        fn object_just_before_card_boundary_not_in_next_card()
        {
            let mut heap = make_heap();
            // Allocate 64 bytes — it won't reach card 1
            alloc_adult_raw(&mut heap, 64);
            // Card 1 should be empty
            let c1 = candidates_for_card(&heap, 1);
            assert!(c1.is_empty(), "small object at start should not appear in card 1");
        }

        #[test]
        fn dead_object_not_in_adult_live_not_found()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr);
            // After dealloc, adult_live is empty so no candidates
            assert!(candidates_for_card(&heap, 0).is_empty());
        }
    }

    // =========================================================================
    // Minor GC — structural correctness (empty stack, no RuntimeType needed)
    // =========================================================================

    mod minor_gc_tests
    {
        use super::*;

        #[test]
        fn flips_active_teen()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            assert_eq!(heap.active_teen, 0);
            heap.collect_minor(&mut stack);
            assert_eq!(heap.active_teen, 1);
        }

        #[test]
        fn double_flip_restores_active_teen()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            heap.collect_minor(&mut stack);
            heap.collect_minor(&mut stack);
            assert_eq!(heap.active_teen, 0);
        }

        #[test]
        fn n_flips_gives_correct_active_teen()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            for i in 0..10usize
            {
                heap.collect_minor(&mut stack);
                assert_eq!(heap.active_teen, (i + 1) % TEEN_COUNT);
            }
        }

        #[test]
        fn clears_dirty_cards()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            // Manually dirty cards — scan_card will find no objects so nothing to evacuate
            heap.card_table[0] = DIRTY;
            heap.card_table[3] = DIRTY;
            heap.collect_minor(&mut stack);
            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn clean_cards_stay_clean()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            heap.collect_minor(&mut stack);
            assert!(heap.card_table.iter().all(|&b| b == CLEAN));
        }

        #[test]
        fn releases_infant_space()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            // Fill infant with several allocations
            let layout = Layout::from_size_align(64, 8).unwrap();
            for _ in 0..10
            {
                heap.infant.raw_alloc(layout);
            }
            heap.collect_minor(&mut stack);
            // After GC, infant should be reset — can allocate again
            assert!(heap.infant.raw_alloc(layout).is_some());
        }

        #[test]
        fn clears_from_teen_live()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let from_idx = heap.active_teen;
            heap.teen_live[from_idx].insert(0, 64);
            heap.collect_minor(&mut stack);
            assert!(heap.teen_live[from_idx].is_empty());
        }

        #[test]
        fn adult_live_unaffected_with_no_promotions()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            alloc_adult_raw(&mut heap, 64);
            heap.collect_minor(&mut stack);
            // No promotions happened (empty young gen) — adult_live unchanged
            assert_eq!(heap.adult_live.len(), 1);
        }

        #[test]
        fn multiple_minor_gcs_on_empty_heap_succeed()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            for _ in 0..10
            {
                heap.collect_minor(&mut stack);
            }
        }
    }

    // =========================================================================
    // Sweep phase / major GC (without RuntimeType — mark objects manually)
    // =========================================================================

    mod sweep_tests
    {
        use super::*;

        #[test]
        fn sweep_empty_adult_live_is_noop()
        {
            let mut heap = make_heap();
            heap.sweep_phase();
            assert!(heap.adult_live.is_empty());
        }

        #[test]
        fn sweep_removes_unmarked_object()
        {
            let mut heap = make_heap();
            alloc_adult_raw(&mut heap, 64);
            heap.sweep_phase(); // object is not marked → swept
            assert_eq!(heap.adult_live.len(), 0);
        }

        #[test]
        fn sweep_preserves_marked_object()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            unsafe { &mut *(ptr.as_ptr() as *mut ObjectHeader) }.set_mark();
            heap.sweep_phase();
            assert_eq!(heap.adult_live.len(), 1);
        }

        #[test]
        fn sweep_clears_mark_on_survivors()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            let header = unsafe { &mut *(ptr.as_ptr() as *mut ObjectHeader) };
            header.set_mark();
            heap.sweep_phase();
            let header = unsafe { &*(ptr.as_ptr() as *const ObjectHeader) };
            assert!(!header.is_marked(), "sweep should clear mark on survivors");
        }

        #[test]
        fn sweep_removes_unmarked_keeps_marked_mixed()
        {
            let mut heap = make_heap();
            let dead1 = alloc_adult_raw(&mut heap, 64);
            let live = alloc_adult_raw(&mut heap, 64);
            let dead2 = alloc_adult_raw(&mut heap, 64);

            unsafe { &mut *(live.as_ptr() as *mut ObjectHeader) }.set_mark();

            heap.sweep_phase();

            assert_eq!(heap.adult_live.len(), 1);
            let live_offset = unsafe { live.byte_offset_from_unsigned(heap.adult_base) };
            assert!(heap.adult_live.contains_key(&live_offset));
            let dead1_offset = unsafe { dead1.byte_offset_from_unsigned(heap.adult_base) };
            let dead2_offset = unsafe { dead2.byte_offset_from_unsigned(heap.adult_base) };
            assert!(!heap.adult_live.contains_key(&dead1_offset));
            assert!(!heap.adult_live.contains_key(&dead2_offset));
        }

        #[test]
        fn sweep_all_marked_all_survive()
        {
            let mut heap = make_heap();
            for _ in 0..5
            {
                let ptr = alloc_adult_raw(&mut heap, 64);
                unsafe { &mut *(ptr.as_ptr() as *mut ObjectHeader) }.set_mark();
            }
            heap.sweep_phase();
            assert_eq!(heap.adult_live.len(), 5);
        }

        #[test]
        fn sweep_all_unmarked_all_removed()
        {
            let mut heap = make_heap();
            for _ in 0..5
            {
                alloc_adult_raw(&mut heap, 64);
            }
            heap.sweep_phase();
            assert_eq!(heap.adult_live.len(), 0);
        }

        #[test]
        fn major_gc_on_empty_heap_succeeds()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            heap.collect_major(&mut stack);
        }

        #[test]
        fn major_gc_sweeps_unreachable_adult()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            alloc_adult_raw(&mut heap, 64);
            heap.collect_major(&mut stack);
            // Object unreachable (not in stack) → swept
            assert_eq!(heap.adult_live.len(), 0);
        }

        #[test]
        fn major_gc_runs_minor_gc_first()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let initial_teen = heap.active_teen;
            heap.collect_major(&mut stack);
            // Minor GC ran as part of major, flipping active_teen
            assert_ne!(heap.active_teen, initial_teen);
        }
    }

    // =========================================================================
    // raw_alloc routing
    // =========================================================================

    mod alloc_routing_tests
    {
        use super::*;

        #[test]
        fn small_object_goes_to_infant()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let layout = Layout::from_size_align(64, 8).unwrap();
            let ptr = heap.raw_alloc(layout, &mut stack).unwrap();
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Infant));
        }

        #[test]
        fn large_object_bypasses_infant_to_adult()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let large = heap.infant.capacity() / Heap::MAX_INFANT_ALLOC_DIVISOR + 1;
            let layout = Layout::from_size_align(large, HEAP_ALIGN).unwrap();
            let ptr = heap.raw_alloc(layout, &mut stack).unwrap();
            assert_eq!(heap.get_pool(ptr), Some(PoolType::Adult));
        }

        #[test]
        fn large_alloc_records_in_adult_live()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let large = heap.infant.capacity() / Heap::MAX_INFANT_ALLOC_DIVISOR + 1;
            let layout = Layout::from_size_align(large, HEAP_ALIGN).unwrap();
            heap.raw_alloc(layout, &mut stack).unwrap();
            assert_eq!(heap.adult_live.len(), 1);
        }

        #[test]
        fn full_infant_triggers_minor_gc()
        {
            let mut heap = make_heap();
            let mut stack = make_stack();
            let initial_teen = heap.active_teen;

            // Exhaust infant space
            let chunk = heap.infant.capacity() / 4;
            let layout = Layout::from_size_align(chunk, HEAP_ALIGN).unwrap();
            while heap.infant.raw_alloc(layout).is_some() {}

            // Next raw_alloc should trigger minor GC
            let small = Layout::from_size_align(64, 8).unwrap();
            heap.raw_alloc(small, &mut stack);

            assert_ne!(heap.active_teen, initial_teen, "minor GC should have run");
        }
    }

    // =========================================================================
    // dealloc
    // =========================================================================

    mod dealloc_tests
    {
        use super::*;

        #[test]
        fn dealloc_adult_removes_from_live()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr);
            assert!(heap.adult_live.is_empty());
        }

        #[test]
        fn dealloc_adult_allows_reuse()
        {
            let mut heap = make_heap();
            let ptr = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr);
            // Should be able to allocate again without error
            let ptr2 = alloc_adult_raw(&mut heap, 64);
            assert_eq!(heap.adult_live.len(), 1);
            let _ = ptr2;
        }

        #[test]
        fn dealloc_infant_is_noop()
        {
            let mut heap = make_heap();
            let ptr = alloc_infant_raw(&mut heap, 64);
            heap.dealloc::<u8>(ptr); // should not panic
        }

        #[test]
        fn dealloc_external_is_noop()
        {
            let mut heap = make_heap();
            let external = NonNull::new(0x1000 as *mut u8).unwrap();
            heap.dealloc::<u8>(external); // should not panic
        }

        #[test]
        fn dealloc_correct_object_among_multiple()
        {
            let mut heap = make_heap();
            let p1 = alloc_adult_raw(&mut heap, 64);
            let p2 = alloc_adult_raw(&mut heap, 64);
            let p3 = alloc_adult_raw(&mut heap, 64);
            heap.dealloc::<u8>(p2);
            assert_eq!(heap.adult_live.len(), 2);
            let off1 = unsafe { p1.byte_offset_from_unsigned(heap.adult_base) };
            let off3 = unsafe { p3.byte_offset_from_unsigned(heap.adult_base) };
            assert!(heap.adult_live.contains_key(&off1));
            assert!(heap.adult_live.contains_key(&off3));
        }

        #[test]
        fn dealloc_all_then_adult_live_empty()
        {
            let mut heap = make_heap();
            let ptrs: Vec<_> = (0..5).map(|_| alloc_adult_raw(&mut heap, 64)).collect();
            for ptr in ptrs
            {
                heap.dealloc::<u8>(ptr);
            }
            assert!(heap.adult_live.is_empty());
        }
    }

    // =========================================================================
    // Regressions — original test suite kept intact
    // =========================================================================

    #[test]
    fn test_align_up()
    {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4095, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_up(4097, 4096), 8192);
        assert_eq!(align_up(8192, 4096), 8192);
        assert_eq!(align_up(123456, 512), 123904);
    }

    #[test]
    fn test_ratio_split_exact()
    {
        let ratio = Ratio(1, 2);
        assert_eq!(ratio.split(300), (100, 200));

        let ratio2 = Ratio(4, 1);
        assert_eq!(ratio2.split(100), (80, 20));
    }

    #[test]
    fn test_ratio_split_imperfect()
    {
        let ratio = Ratio(4, 1);
        assert_eq!(ratio.split(10), (8, 2));
        assert_eq!(ratio.split(11), (8, 3));
        assert_eq!(ratio.split(0), (0, 0));

        let ratio_large = Ratio(99, 1);
        assert_eq!(ratio_large.split(1000), (990, 10));
    }

    #[test]
    fn test_object_header_initial_state()
    {
        let header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };
        assert!(!header.is_forwarded());
        assert_eq!(header.age(), 0);
    }

    #[test]
    fn test_object_header_age_increment_and_saturation()
    {
        let mut header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        header.increment_age();
        assert_eq!(header.age(), 1);
        assert!(!header.is_forwarded(), "Age increment should not trigger forwarded bit");

        for _ in 0..20
        {
            header.increment_age();
        }

        assert_eq!(header.age(), 15);
    }

    #[test]
    fn test_object_header_preserved_bits()
    {
        let mut header = ObjectHeader {
            mark_word: 0b11111111_11111111_11111111_11100000,
            vtable_or_type: NonNull::dangling(),
            size: 64,
        };

        assert_eq!(header.age(), 0);
        header.increment_age();
        assert_eq!(header.age(), 1);

        let high_bits = header.mark_word & !0x1F;
        assert_eq!(high_bits, 0b11111111_11111111_11111111_11100000);
    }

    #[test]
    fn test_object_header_forwarding()
    {
        let mut header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        let target_addr = NonNull::new(0xABCD_1200 as *mut u8).unwrap();
        header.set_forwarding_address(target_addr);

        assert!(header.is_forwarded());
        assert_eq!(header.forwarding_address(), target_addr);
    }

    #[test]
    #[should_panic(expected = "called forwarding_address on a live object")]
    fn test_forwarding_address_on_live_object_panics()
    {
        let header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        let _ = header.forwarding_address();
    }

    #[test]
    fn test_heap_initialization_metrics()
    {
        let heap = Heap::with_capacity(TEST_HEAP_SIZE).unwrap();

        assert_eq!(heap.active_teen, 0);
        assert!(!heap.card_table.is_empty());
        assert!(heap.card_table.iter().all(|&status| status == CLEAN));
    }

    #[test]
    fn test_card_index_out_of_bounds()
    {
        let heap = Heap::with_capacity(TEST_HEAP_SIZE).unwrap();
        let external_ptr = NonNull::new(0x1000 as *mut u8).unwrap();

        assert_eq!(heap.card_index_of(external_ptr), None);
    }

    #[test]
    fn test_generation_classification_isolation()
    {
        let heap = Heap::with_capacity(TEST_HEAP_SIZE).unwrap();
        let external_ptr = NonNull::new(0x5000 as *mut u8).unwrap();

        assert_eq!(heap.get_pool(external_ptr), None);
        assert!(!heap.is_youth(external_ptr));
    }

    #[test]
    fn test_card_offset_math()
    {
        let base_addr = 0x10000;
        let obj_addr = 0x10100;
        let size = 600;

        let start_card = (obj_addr - base_addr) / CARD_SIZE;
        let end_card = (obj_addr + size - 1 - base_addr) / CARD_SIZE;

        assert_eq!(start_card, 0);
        assert_eq!(end_card, 1);

        let card_1_start = base_addr + 1 * CARD_SIZE;
        let offset_for_card_1 = card_1_start - obj_addr;
        assert_eq!(offset_for_card_1, 256);
    }
}
