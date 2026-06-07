use std::{
    alloc::{Layout, LayoutError, alloc}, array::from_fn, collections::BTreeMap, mem::transmute, ptr::NonNull
};

use crate::memory::{
    allocators::{AllocatorError, arena::ArenaAllocator, general::GeneralAllocator}, datumspace::tables::types::{RuntimeType, RuntimeTypeKind}, stack::{Stack, entry::StackEntry}
};

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
    const DEAD_BIT: usize = 1 << 5;

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

    pub fn is_dead(&self) -> bool
    {
        (self.mark_word & Self::DEAD_BIT) != 0
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

    /// Maps each card index to the byte offset from the start of the card
    /// back to the closest valid `ObjectHeader` starting at or before it.
    card_offsets: Vec<usize>,

    adult_live: BTreeMap<usize, usize>,
}

impl Heap
{
    /// Objects larger than `infant.capacity() / MAX_INFANT_ALLOC_DIVISOR`
    /// bypass the infant space and go straight to the adult gen.
    const MAX_INFANT_ALLOC_DIVISOR: usize = 2;

    /// Objects that survive this many minor GCs are promoted to the adult gen.
    const ADULT_THRESHOLD: usize = 8;

    pub fn with_capacity(capacity: usize) -> Result<Self, HeapError>
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

        // init cards.
        let card_offsets = vec![usize::MAX; num_cards];

        Ok(Self {
            base,
            layout,
            infant,
            teen,
            active_teen: 0,
            adult,
            adult_base,
            card_table,
            card_offsets,
            adult_live: BTreeMap::new(),
        })
    }

    /// Allocates an object in the adult generation and records its boundary mapping.
    fn alloc_adult(&mut self, layout: Layout) -> Option<ObjRef>
    {
        let ptr = self.adult.raw_alloc(layout)?;
        let base = self.adult_base.as_ptr() as usize;
        let offset = ptr.as_ptr() as usize - base;
        self.adult_live.insert(offset, layout.size());

        // card_offsets update
        let start_card = offset / CARD_SIZE;
        let end_card = (offset + layout.size() - 1) / CARD_SIZE;
        if self.card_offsets[start_card] == usize::MAX
        {
            self.card_offsets[start_card] = offset;
        }

        for card_idx in (start_card + 1)..=end_card
        {
            self.card_offsets[card_idx] = offset;
        }

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
                let header = &*(parent_ptr.as_ptr() as *const ObjectHeader);
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
                        if parent_is_adult && self.is_youth(new_child) {
                            if let Some(field_ref) = NonNull::new(field_ptr.cast::<u8>())
                                && let Some(card_idx) = self.card_index_of(field_ref)
                            {
                                self.card_table[card_idx] = DIRTY;
                            }
                        }
                    }
                }
            }
        }

        // remove all remaining infants
        self.infant.release_all();

        // purge half teen space
        self.teen[from_teen_idx].release_all();

        self.active_teen = to_teen_idx;
    }

    /// Evacuates a single live object out of young gen.
    unsafe fn evacuate(&mut self, obj_ptr: ObjRef, to_teen_idx: usize, worklist: &mut Vec<ObjRef>) -> ObjRef
    {
        let header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };

        if header.is_forwarded()
        {
            return header.forwarding_address();
        }

        let (offsets, size) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };

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
            self.teen[to_teen_idx].raw_alloc(layout).unwrap_or_else(|| {
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
        let start_offset = self.card_offsets[card_idx];
        if start_offset == usize::MAX { return; }

        let card_end_offset = (card_idx + 1) * CARD_SIZE;

        // Collect live objects that start at or before this card and could overlap it.
        // The BTreeMap range gives us objects in ascending order, which is correct for
        // the worklist — we just need all objects overlapping [card_start, card_end).
        let candidates: Vec<usize> = self.adult_live
            .range(start_offset..card_end_offset)
            .map(|(&off, _)| off)
            .collect();

        for obj_offset in candidates
        {
            let obj_ptr: ObjRef = unsafe {
                NonNull::new_unchecked(self.adult_base.as_ptr().add(obj_offset))
            };
            let header = unsafe { &*(obj_ptr.as_ptr() as *const ObjectHeader) };

            if header.is_forwarded() { continue; }

            let (offsets, _) = unsafe { Self::gc_layout(header.vtable_or_type, obj_ptr) };
            let offsets: Vec<usize> = offsets.to_vec();

            for offset in offsets
            {
                let field_ptr: FieldPtr = unsafe { obj_ptr.as_ptr().add(offset).cast() };
                let child_ptr = unsafe { *field_ptr };

                if self.is_youth(child_ptr)
                {
                    let new_child = unsafe { self.evacuate(child_ptr, to_teen_idx, worklist) };
                    unsafe { *field_ptr = new_child; }

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
        if !self.adult.contains(ptr)
        {
            return None;
        }
        let offset = (ptr.as_ptr() as usize).checked_sub(self.adult_base.as_ptr() as usize)?;
        Some(offset / CARD_SIZE)
    }

    unsafe fn gc_layout(
        vtable: NonNull<u8>,
        obj_base: ObjRef
    ) -> (&'static [usize], usize)
    {
        let ty = unsafe { vtable.cast::<RuntimeType>().as_ref() };
        let page = unsafe { ty.back_pointer.as_ref().get_page() };

        match ty.kind {
            RuntimeTypeKind::Struct { instance_size, gc_offsets_index, gc_offsets_count, .. } => {
                let start = gc_offsets_index as usize;
                let offsets = &page.gc_offsets[start..start + gc_offsets_count as usize];
                (unsafe { transmute(offsets) }, instance_size)
            }
            RuntimeTypeKind::Enum { instance_size, variants_index, variants_count, .. } => {
                let tag = unsafe { obj_base.byte_add(size_of::<ObjectHeader>()).cast::<u32>().read() };
                let start = variants_index as usize;
                let variants = &page.enum_variants[start..start + variants_count as usize];
                let variant = variants.iter().find(|v| v.tag == tag)
                    .expect("GC: unknown enum tag — heap corrupted");
                let offsets = page.get_variant_gc_offsets(variant)
                    .expect("GC: invalid gc_offsets range in variant");
                (unsafe { transmute(offsets) }, instance_size)
            }
            RuntimeTypeKind::Imported { .. } => {
                todo!("Whats the plan here")
            }
        }
    }
}

#[cfg(test)]
mod tests
{
    use std::ptr::NonNull;

    use super::*;

    // Plentiful, clean heap size to satisfy power-of-two suballocations easily
    const TEST_HEAP_SIZE: usize = 64 * 1024 * 1024;

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
        assert_eq!(heap.card_table.len(), heap.card_offsets.len());
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
