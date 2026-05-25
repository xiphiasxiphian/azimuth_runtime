use std::{
    alloc::{Layout, LayoutError, alloc},
    array::from_fn,
    ptr::NonNull,
};

use crate::memory::{
    allocators::{AllocatorError, arena::ArenaAllocator, general::GeneralAllocator},
    stack::{Stack, entry::StackEntry},
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

#[derive(Clone, Copy, Debug)]
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

pub trait Traceable
{
    /// Returns byte offsets (from the object base pointer) of every field
    /// that holds an `ObjRef`.
    fn references(&self) -> &[usize];

    /// Total byte size of this allocation, including `ObjectHeader`.
    fn size(&self) -> usize;
}

// ---------------------------------------------------------------------------
// Heap
// ---------------------------------------------------------------------------

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
        // Compute raw region sizes.
        let (young_raw, old_raw) = YOUNG_OLD_RATIO.split(capacity);
        let (infant_raw, teen_total_raw) = INFANT_TEEN_RATIO.split(young_raw);
        let teen_raw = teen_total_raw / TEEN_COUNT;

        // Align every region to a page boundary.
        let infant_capacity = align_up(infant_raw, HEAP_ALIGN);
        let teen_capacity = align_up(teen_raw, HEAP_ALIGN); // per teen space
        let adult_capacity = align_up(old_raw, HEAP_ALIGN);

        let total_teen_capacity = teen_capacity * TEEN_COUNT;
        let total_capacity = infant_capacity + total_teen_capacity + adult_capacity;

        // Single contiguous allocation for the whole heap.
        let layout = Layout::from_size_align(total_capacity, HEAP_ALIGN).map_err(HeapError::InvalidLayout)?;

        let base = NonNull::new(unsafe { alloc(layout) })
            .ok_or(HeapError::CannotProvision(AllocatorError::FailedInitialAllocation))?;

        // Carve out sub-regions from the slab.
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

        // One card-table entry per CARD_SIZE bytes of adult space.
        let card_table = vec![CLEAN; adult_capacity / CARD_SIZE];

        Ok(Self {
            base,
            layout,
            infant,
            teen,
            active_teen: 0,
            adult,
            adult_base,
            card_table,
        })
    }

    fn raw_alloc(&mut self, layout: Layout, stack: &mut Stack) -> Option<ObjRef>
    {
        // large objects skip the infant space entirely.
        if layout.size() > self.infant.capacity() / Self::MAX_INFANT_ALLOC_DIVISOR
        {
            return self.adult.raw_alloc(layout);
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

        // TODO: When to perform major GC

        // try and allocate into adult if everything else fails
        self.adult.raw_alloc(layout)
    }

    fn alloc<T>(&mut self, value: T, stack: &mut Stack) -> Option<NonNull<T>>
    {
        self.raw_alloc(Layout::new::<T>(), stack)
            .map(|x| x.cast::<T>())
            .inspect(|x| unsafe {
                x.write(value);
            })
    }

    pub fn dealloc<T>(&mut self, ptr: NonNull<T>)
    {
        match self.get_pool(ptr.cast())
        {
            None | Some(PoolType::Infant) =>
            {} // All infant space just gets deallocated at once anyway
            Some(PoolType::Teen(i)) => self.teen[i].dealloc(ptr),
            Some(PoolType::Adult) => self.adult.dealloc(ptr),
        }
    }

    /// Must be called on every reference-field write: `obj.field = new_value`.
    ///
    /// When an old-gen object acquires a pointer into the young gen the
    /// containing card is marked dirty so the minor GC can find that root
    /// without scanning the entire old gen.
    pub fn write_barrier(&mut self, parent_ptr: ObjRef, field_addr: FieldPtr, new_value: ObjRef)
    {
        // perform the actual write
        unsafe {
            *field_addr = new_value;
        }

        // Record the cross-generational pointer.
        if !self.is_youth(parent_ptr)
            && self.is_youth(new_value)
            && let Some(card_idx) = self.card_index_of(parent_ptr)
        {
            self.card_table[card_idx] = DIRTY;
        }
    }

    /// Scavenges the infant space and the active teen ("from") space.
    ///
    /// Live objects are evacuated into the inactive teen ("to") space.
    /// Objects that have survived `ADULT_THRESHOLD` collections, or for which
    /// the teen "to" space is full, are promoted directly to the adult gen.
    ///
    /// After a successful minor GC:
    /// - the infant space is empty (bump pointer reset),
    /// - the "from" teen space is empty,
    /// - `self.active_teen` flips to the newly populated "to" space.
    pub fn collect_minor(&mut self, stack: &mut Stack)
    {
        let from_teen_idx = self.active_teen;
        let to_teen_idx = 1 - self.active_teen;

        let mut worklist: Vec<ObjRef> = Vec::new();

        for entry in stack.iter_mut()
        {
            if let StackEntry::Reference(Some(obj_ptr)) = entry
            {
                if self.is_youth(*obj_ptr)
                {
                    *obj_ptr = unsafe { self.evacuate(*obj_ptr, to_teen_idx, &mut worklist) };
                }
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
            let card_start = unsafe { self.adult_base.as_ptr().add(card_idx * CARD_SIZE) };
            self.scan_card(card_start, to_teen_idx, &mut worklist);
        }

        while let Some(parent_ptr) = worklist.pop()
        {
            unsafe {
                let header = &*(parent_ptr.as_ptr() as *const ObjectHeader);
                let metadata = self.get_metadata(header.vtable_or_type);
                let offsets: Vec<usize> = metadata.references().to_vec();

                for offset in offsets
                {
                    let field_ptr: FieldPtr = parent_ptr.as_ptr().add(offset).cast();
                    let child_ptr = *field_ptr;

                    if self.is_youth(child_ptr)
                    {
                        *field_ptr = self.evacuate(child_ptr, to_teen_idx, &mut worklist);
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
    ///
    /// Returns the new address of the object.  If the object was already
    /// moved in this GC cycle (i.e. it is reachable via multiple paths) the
    /// forwarding address stored in the old header is returned immediately.
    ///
    /// Destination policy:
    /// - age >= `ADULT_THRESHOLD` -> adult gen (normal promotion).
    /// - teen "to" space has room -> teen "to" space (age incremented).
    /// - teen "to" space is full -> adult gen (emergency / overflow promotion).
    ///
    /// The new object is pushed onto `worklist` so its own reference fields
    /// are traced in phase 3 of the minor GC.
    ///
    /// # Safety
    /// `obj_ptr` must point to a valid, live `ObjectHeader` in young gen.
    unsafe fn evacuate(&mut self, obj_ptr: ObjRef, to_teen_idx: usize, worklist: &mut Vec<ObjRef>) -> ObjRef
    {
        // safety: caller guarantees obj_ptr is a valid young-gen object.
        let header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };

        // follow forward pointer as has already been evacuated
        if header.is_forwarded()
        {
            return header.forwarding_address();
        }

        let metadata = unsafe { self.get_metadata(header.vtable_or_type) };
        let size = metadata.size();

        // TODO: Figure how the error for these allocation fails will work

        // using objectheader alignment to not waste space
        let layout = Layout::from_size_align(size, align_of::<ObjectHeader>())
            .expect("Object metadata returned invalid size or alignment");

        let should_promote = header.age() >= Self::ADULT_THRESHOLD;
        let destination: ObjRef = if should_promote
        {
            // promote
            self.adult
                .raw_alloc(layout)
                .expect("OOM in old gen during normal promotion")
        }
        else
        {
            // copy to survivor space
            self.teen[to_teen_idx].raw_alloc(layout).unwrap_or_else(|| {
                self.adult
                    .raw_alloc(layout)
                    .expect("OOM in old gen during overflow promotion")
            })
        };

        unsafe {
            destination.copy_from_nonoverlapping(obj_ptr, size);
        }

        // Increment the age in the new copy (only for teen destinations
        // promoted objects' ages are irrelevant once in the old gen).
        if !should_promote
        {
            let new_header: &mut ObjectHeader = unsafe { destination.cast().as_mut() };
            new_header.increment_age();
        }

        // leave a forwarding pointer in the old copy so that any subsequent
        // references to the same object are redirected to its new location.
        // (The old header is now logically dead.)
        let old_header: &mut ObjectHeader = unsafe { obj_ptr.cast().as_mut() };
        old_header.set_forwarding_address(destination);

        // Schedule the new copy for field-tracing in the transitive closure phase.
        worklist.push(destination);

        destination
    }

    /// Scans one card-sized region of the old gen, evacuating any young-gen
    /// pointers found in reference fields of live objects within that region.
    ///
    /// Objects are walked sequentially using `ObjectHeader::size` as the stride.
    /// A zero-size header (unallocated / padding) terminates the scan early.
    ///
    /// # Safety
    /// `card_start` must be card-aligned and point into the adult allocator's
    /// live memory range.
    fn scan_card(&mut self, card_start: *mut u8, to_teen_idx: usize, worklist: &mut Vec<ObjRef>)
    {
        let card_end = unsafe { card_start.add(CARD_SIZE) };
        let mut cursor = card_start;

        while cursor < card_end
        {
            // SAFETY: cursor stays within [card_start, card_end) and we
            // advance by header.size each iteration.
            let obj_ptr = unsafe { NonNull::new_unchecked(cursor as *mut ObjectHeader) };
            let header = unsafe { obj_ptr.as_ref() };

            let size = header.size;
            if size == 0
            {
                break;
            }

            // Trace reference fields of live (non-forwarded) objects only.
            // Forwarded objects are dead; their fields have already been fixed.
            if !header.is_forwarded()
            {
                let metadata = unsafe { self.get_metadata(header.vtable_or_type) };
                let offsets: Vec<usize> = metadata.references().to_vec();

                for offset in offsets
                {
                    let field_ptr: FieldPtr = unsafe { cursor.add(offset).cast() };
                    let child_ptr = unsafe { *field_ptr };

                    if self.is_youth(child_ptr)
                    {
                        unsafe {
                            *field_ptr = self.evacuate(child_ptr, to_teen_idx, worklist);
                        }
                    }
                }
            }

            cursor = unsafe { cursor.add(size) };
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
    ///
    /// Returns `None` if `ptr` does not lie inside the adult region (e.g. if
    /// called on a young-gen pointer, which should never happen via the normal
    /// write-barrier path but is safe to handle gracefully).
    fn card_index_of(&self, ptr: ObjRef) -> Option<usize>
    {
        if !self.adult.contains(ptr)
        {
            return None;
        }
        let offset = (ptr.as_ptr() as usize).checked_sub(self.adult_base.as_ptr() as usize)?;
        Some(offset / CARD_SIZE)
    }

    /// Looks up the `Traceable` metadata for an object via its vtable pointer.
    ///
    /// # Safety
    /// `vtable` must be a valid pointer returned by the type system at object
    /// allocation time.
    unsafe fn get_metadata(&self, _vtable: NonNull<u8>) -> &dyn Traceable
    {
        todo!("Waiting for type system setup")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::NonNull;

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4095, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_up(4097, 4096), 8192);
    }

    #[test]
    fn test_ratio_split_exact() {
        let ratio = Ratio(1, 2);
        assert_eq!(ratio.split(300), (100, 200));

        let ratio2 = Ratio(4, 1);
        assert_eq!(ratio2.split(100), (80, 20));
    }

    #[test]
    fn test_ratio_split_imperfect() {
        let ratio = Ratio(4, 1);
        // 10 * 4 / 5 = 8. 10 - 8 = 2.
        assert_eq!(ratio.split(10), (8, 2));
        // 11 * 4 / 5 = 8. 11 - 8 = 3. Ensures no dropped bytes.
        assert_eq!(ratio.split(11), (8, 3));
    }


    #[test]
    fn test_object_header_initial_state() {
        let header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };
        assert!(!header.is_forwarded());
        assert_eq!(header.age(), 0);
    }

    #[test]
    fn test_object_header_age_increment_and_saturation() {
        let mut header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        header.increment_age();
        assert_eq!(header.age(), 1);
        assert!(!header.is_forwarded(), "Age increment should not trigger forwarded bit");

        // Force saturation
        for _ in 0..20 {
            header.increment_age();
        }

        // Age should cap at 15 (0x0F) according to AGE_MASK
        assert_eq!(header.age(), 15);
    }

    #[test]
    fn test_object_header_forwarding() {
        let mut header = ObjectHeader {
            mark_word: 0,
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        // Create a dummy forwarding address
        let target_addr = NonNull::new(0xABCD_1200 as *mut u8).unwrap();
        header.set_forwarding_address(target_addr);

        assert!(header.is_forwarded());
        assert_eq!(header.forwarding_address(), target_addr);
    }

    #[test]
    #[should_panic(expected = "called forwarding_address on a live object")]
    fn test_forwarding_address_on_live_object_panics() {
        let header = ObjectHeader {
            mark_word: 0, // Not forwarded
            vtable_or_type: NonNull::dangling(),
            size: 32,
        };

        // This should panic
        let _ = header.forwarding_address();
    }
}
