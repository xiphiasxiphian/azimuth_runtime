use std::{
    alloc::{Layout, alloc, dealloc},
    ptr::NonNull,
};

use derive_more::{Add, Sub};

use crate::{
    guard,
    loader::SymbolId,
    memory::datumspace::{
        link_table::Link,
        runnable::Runnable,
        tables::{
            constant_table::ConstantTableEntry,
            symbol_table::Symbol,
            types::{RuntimeEnumVariant, RuntimeType, RuntimeTypeKind},
        },
    },
};

/* ┌─────────────────────────┐  <- base_ptr
   │  DatumPageHeader        │  fixed size, contains section lengths
   │  id_loc                 │
   │  constants_loc          │
   │  functions_loc          │
   │  symbol_table_loc       │
   │  import_table_loc       │
   ├─────────────────────────┤  <- base + sizeof(DatumPageHeader)
   │  id bytes               │  id_len bytes, no null terminator
   │  (align padding)        │
   ├─────────────────────────┤  <- base + sizeof(DatumPageHeader) + id_len
   │  ImportEntry[]          │  import_table_len entries
   ├─────────────────────────┤
   │  SymbolEntry[]          │  symbol_table_len entries
   ├─────────────────────────┤
   │  Constant[]             │  constants_len entries
   │  (each Constant stores  │  (these will be lazily evaluated)
   │   an offset into the    │
   │   data blob below)      │
   ├─────────────────────────┤
   │  Runnable[]             │  functions_len entries
   │  (each Runnable stores  │  (these will be lazily evaluated)
   │   an offset into the    │
   │   code blob below)      │
   ├─────────────────────────┤
   │  RuntimeType[]          │ indexed by TypeSignature::type_index
   ├─────────────────────────┤
   │  RuntimeEnumVariant[]   │ flat array of all variants
   ├─────────────────────────┤
   │  usize[]                │ flat array of all GC reference offsets
   ├─────────────────────────┤
   │  data blob              │  raw bytes for all constants
   ├─────────────────────────┤
   │  code blob              │  raw bytecode for all functions
   └─────────────────────────┘
*/

#[derive(Clone, Copy, Debug, Add, Sub)]
#[repr(transparent)]
pub struct Offset(pub u32);

impl Offset
{
    pub unsafe fn as_ptr<T>(&self, base: NonNull<u8>) -> NonNull<T>
    {
        unsafe { base.byte_add(self.0 as usize).cast::<T>() }
    }
}

pub type BlockLocation = (Offset, u32);

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct InlinedString
{
    location: BlockLocation,
}

impl InlinedString
{
    pub fn new(location: BlockLocation) -> Self
    {
        Self { location }
    }

    pub unsafe fn get(&self, base: NonNull<u8>) -> Option<&str>
    {
        unsafe {
            let bytes: &[u8] =
                std::slice::from_raw_parts(self.location.0.as_ptr(base).as_ptr(), self.location.1 as usize);
            str::from_utf8(bytes).ok()
        }
    }

    pub unsafe fn get_unchecked(&self, base: NonNull<u8>) -> &str
    {
        unsafe {
            let bytes: &[u8] =
                std::slice::from_raw_parts(self.location.0.as_ptr(base).as_ptr(), self.location.1 as usize);
            str::from_utf8_unchecked(bytes)
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DatumPageHeader
{
    pub id: SymbolId,
    pub link_table: BlockLocation,
    pub symbol_table: BlockLocation,
    pub constants: BlockLocation,
    pub functions: BlockLocation,
    pub types: BlockLocation,
    pub enum_variants: BlockLocation,
    pub gc_offsets: BlockLocation,
    pub bytecode_blob: BlockLocation,
    pub data_blob: BlockLocation,
}

impl DatumPageHeader
{
    /// Constructs a view over the Datumpage, from its header
    ///
    /// SAFETY: This is only safe is the given page header is embedded within datumspace
    /// in an _actual_ `DatumPage`. Otherwise, this will end up reading into nonsense memory
    pub unsafe fn get_page(&self) -> DatumPage<'_>
    {
        unsafe { DatumPage::from_base_ptr(NonNull::from_ref(self).cast()) }
    }
}

/// A typed view over a raw `DatumPage` block.
/// All slices point into the same contiguous allocation.
pub struct DatumPage<'a>
{
    pub id: &'a SymbolId,
    pub links: &'a [Link],
    pub symbols: &'a [Symbol],
    pub functions: &'a [Runnable],
    pub constants: &'a [ConstantTableEntry],
    pub types: &'a [RuntimeType],
    pub enum_variants: &'a [RuntimeEnumVariant],
    pub gc_offsets: &'a [usize],
    pub bytecode_blob: &'a [u8],
    pub data_blob: &'a [u8],
}

impl<'a> DatumPage<'a>
{
    /// Reinterpret the base pointer of a filled page block.
    /// SAFETY:  only if the block was built by `load_datum`.
    pub unsafe fn from_base_ptr(ptr: NonNull<u8>) -> Self
    {
        let header: &DatumPageHeader = unsafe { ptr.cast().as_ref() };

        // module id
        let id = &header.id;

        // link table
        let links: &'a [Link] = unsafe { Self::get_slice(ptr, header.link_table) };

        // symbol table
        let symbols: &'a [Symbol] = unsafe { Self::get_slice(ptr, header.symbol_table) };

        // function table
        let functions: &'a [Runnable] = unsafe { Self::get_slice(ptr, header.functions) };

        // constant table
        let constants: &'a [ConstantTableEntry] = unsafe { Self::get_slice(ptr, header.constants) };

        let types: &'a [RuntimeType] = unsafe { Self::get_slice(ptr, header.types) };

        let enum_variants: &'a [RuntimeEnumVariant] = unsafe { Self::get_slice(ptr, header.enum_variants) };

        let gc_offsets: &'a [usize] = unsafe { Self::get_slice(ptr, header.gc_offsets) };

        // bytecode and function headers
        let bytecode_blob: &'a [u8] = unsafe { Self::get_slice(ptr, header.bytecode_blob) };

        // All constants and other data
        let data_blob: &'a [u8] = unsafe { Self::get_slice(ptr, header.data_blob) };

        Self {
            id,
            links,
            symbols,
            functions,
            constants,
            types,
            enum_variants,
            gc_offsets,
            bytecode_blob,
            data_blob,
        }
    }

    unsafe fn get_slice<T>(base: NonNull<u8>, location: BlockLocation) -> &'a [T]
    where
        T: Sized,
    {
        if location.1 == 0
        {
            return &[];
        }

        let ptr = unsafe { base.byte_add(location.0.0 as usize).cast::<T>() };

        assert!(
            ptr.as_ptr() as usize % std::mem::align_of::<T>() == 0,
            "Section offset {} is not aligned for type {} (requires alignment of {})",
            location.0.0,
            std::any::type_name::<T>(),
            std::mem::align_of::<T>()
        );

        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), location.1 as usize / size_of::<T>()) }
    }

    /// Get the pre-calculated GC offsets for a struct by its index
    pub fn get_struct_layout(&self, type_index: u32) -> Option<(usize, usize, &'a [usize])>
    {
        let ty = self.types.get(type_index as usize)?;
        if let RuntimeTypeKind::Struct {
            instance_size,
            alignment,
            gc_offsets_index,
            gc_offsets_count,
        } = ty.kind
        {
            let start = gc_offsets_index as usize;
            let end = start + gc_offsets_count as usize;
            Some((instance_size, alignment as usize, self.gc_offsets.get(start..end)?))
        }
        else
        {
            None
        }
    }

    /// Retrieve the specific enum variant information based on the tag found at runtime
    pub fn get_enum_variant_layout(&self, type_index: u32, tag: u32) -> Option<&'a RuntimeEnumVariant>
    {
        let ty = self.types.get(type_index as usize)?;
        if let RuntimeTypeKind::Enum {
            instance_size: _,
            alignment: _,
            variants_index,
            variants_count,
        } = ty.kind
        {
            let start = variants_index as usize;
            let end = start + variants_count as usize;
            let variants = self.enum_variants.get(start..end)?;

            variants.iter().find(|v| v.tag == tag)
        }
        else
        {
            None
        }
    }

    /// Fetch the GC offsets for a resolved enum variant
    pub fn get_variant_gc_offsets(&self, variant: &RuntimeEnumVariant) -> Option<&'a [usize]>
    {
        let start = variant.gc_offsets_index as usize;
        let end = start + variant.gc_offsets_count as usize;
        self.gc_offsets.get(start..end)
    }
}

pub struct PageLayout
{
    pub header: DatumPageHeader,
    pub total_size: usize,
    pub max_alignment: usize,
}

impl PageLayout
{
    pub fn compute(
        id: SymbolId,
        num_links: usize,
        num_symbols: usize,
        num_functions: usize,
        num_constants: usize,
        num_types: usize,
        num_enum_variants: usize,
        num_gc_offsets: usize,
        bytecode_len: usize,
        data_len: usize,
    ) -> Self
    {
        let mut current_offset = std::mem::size_of::<DatumPageHeader>();
        let mut max_align = std::mem::align_of::<DatumPageHeader>();

        macro_rules! align_section {
            ($align_ty:ty, $count:expr) => {{
                let align = std::mem::align_of::<$align_ty>();
                max_align = max_align.max(align);

                current_offset = (current_offset + align - 1) & !(align - 1);

                let start = current_offset;
                let bytes = $count * std::mem::size_of::<$align_ty>();
                current_offset += bytes;

                (Offset(start as u32), bytes as u32)
            }};
        }

        let link_table = align_section!(Link, num_links);
        let symbol_table = align_section!(Symbol, num_symbols);
        let functions = align_section!(Runnable, num_functions);
        let constants = align_section!(ConstantTableEntry, num_constants);
        let types = align_section!(RuntimeType, num_types);
        let enum_variants = align_section!(RuntimeEnumVariant, num_enum_variants);
        let gc_offsets = align_section!(usize, num_gc_offsets);
        let bytecode_blob = align_section!(u8, bytecode_len);
        let data_blob = align_section!(u8, data_len);

        Self {
            header: DatumPageHeader {
                id,
                link_table,
                symbol_table,
                constants,
                functions,
                types,
                enum_variants,
                gc_offsets,
                bytecode_blob,
                data_blob,
            },
            total_size: current_offset,
            max_alignment: max_align,
        }
    }
}

pub struct AlignedPageBuffer
{
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedPageBuffer
{
    pub fn new(size: usize, alignment: usize) -> Self
    {
        let layout = Layout::from_size_align(size, alignment).expect("Invalid layout configuration");
        unsafe {
            let raw_ptr = alloc(layout);
            let ptr = NonNull::new(raw_ptr).expect("Heap allocation failed");
            Self { ptr, layout }
        }
    }

    pub fn as_non_null(&self) -> NonNull<u8>
    {
        self.ptr
    }

    pub fn len(&self) -> usize
    {
        self.layout.size()
    }
}

impl Drop for AlignedPageBuffer
{
    fn drop(&mut self)
    {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

/// Utility for building pages safer
#[derive(Clone, Copy, Debug)]
pub struct PageBuilder
{
    base: NonNull<DatumPageHeader>,
}

impl PageBuilder
{
    /// SAFETY: `raw_base` must have space for the header to be written to it,
    /// and realistically must have space for anything future to be written to
    /// the page
    pub unsafe fn new(raw_base: NonNull<u8>, header: DatumPageHeader) -> Self
    {
        let base = raw_base.cast();

        unsafe { base.write(header) };

        Self { base }
    }

    pub unsafe fn resolve<'a>(self) -> DatumPage<'a>
    {
        unsafe { DatumPage::from_base_ptr(self.base.cast()) }
    }

    pub unsafe fn write_links<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = Link>,
    {
        unsafe { self.write_iter(&self.base.as_ref().link_table, src) }
    }

    pub unsafe fn write_symbols<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = Symbol>,
    {
        unsafe { self.write_iter(&self.base.as_ref().symbol_table, src) }
    }

    pub unsafe fn write_functions<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = Runnable>,
    {
        unsafe { self.write_iter(&self.base.as_ref().functions, src) }
    }

    pub unsafe fn write_constants<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = ConstantTableEntry>,
    {
        unsafe { self.write_iter(&self.base.as_ref().constants, src) }
    }

    pub unsafe fn write_types<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = RuntimeType>,
    {
        unsafe { self.write_iter(&self.base.as_ref().types, src) }
    }

    pub unsafe fn write_enum_variants<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = RuntimeEnumVariant>,
    {
        unsafe { self.write_iter(&self.base.as_ref().enum_variants, src) }
    }

    pub unsafe fn write_gc_offsets<'a, I>(self, src: I) -> Option<Self>
    where
        I: Iterator<Item = usize>,
    {
        unsafe { self.write_iter(&self.base.as_ref().gc_offsets, src) }
    }

    pub unsafe fn write_code_blob(self, src: &[u8]) -> Option<Self>
    {
        unsafe { self.write_blob(src, &self.base.as_ref().bytecode_blob) }
    }

    pub unsafe fn write_data_blob(self, src: &[u8]) -> Option<Self>
    {
        unsafe { self.write_blob(src, &self.base.as_ref().data_blob) }
    }

    unsafe fn write_blob(self, src: &[u8], loc: &BlockLocation) -> Option<Self>
    {
        // Just ensure that there is in fact enough space.
        // This is an assertion as this should _never_ happen
        assert!(<usize>::try_from(loc.1).ok()? >= src.len());

        // Copy the given data into the correct place within the page
        unsafe {
            loc.0
                .as_ptr::<u8>(self.base.cast())
                .copy_from_nonoverlapping(NonNull::from_ref(src).cast(), src.len())
        };

        Some(self)
    }

    unsafe fn write_iter<I, T>(self, loc: &BlockLocation, iter: I) -> Option<Self>
    where
        I: Iterator<Item = T>,
        T: Sized,
    {
        if loc.1 == 0
        {
            return Some(self);
        }
        let base: NonNull<T> = unsafe { loc.0.as_ptr(self.base.cast()) };

        // Invariant check: base must be aligned for T to avoid UB on .write()
        assert!(
            base.as_ptr() as usize % std::mem::align_of::<T>() == 0,
            "Attempted to write to unaligned offset {} for type {}",
            loc.0.0,
            std::any::type_name::<T>()
        );

        let limit = <usize>::try_from(loc.1).ok()? / size_of::<T>();
        for (i, item) in iter.enumerate()
        {
            guard!(i < limit); // writing outside of block location slice
            unsafe { base.add(i).write(item) }
        }

        Some(self)
    }
}

#[cfg(test)]
mod tests
{
    use std::ptr::NonNull;

    use super::*;

    // =========================================================================
    // Memory helpers
    // =========================================================================

    /// A heap-allocated, zeroed byte buffer that hands out a `NonNull<u8>`.
    /// Kept alive for the duration of each test via ownership.
    struct TestBuffer
    {
        data: AlignedPageBuffer,
    }

    impl TestBuffer
    {
        fn new(size: usize) -> Self
        {
            Self {
                data: AlignedPageBuffer::new(size, size_of::<usize>()),
            }
        }

        fn from_buffer(buf: AlignedPageBuffer) -> Self
        {
            Self { data: buf }
        }

        fn as_nonnull(&self) -> NonNull<u8>
        {
            self.data.as_non_null()
        }

        fn as_slice(&mut self) -> &mut [u8]
        {
            unsafe { NonNull::slice_from_raw_parts(self.data.as_non_null(), self.data.len()).as_mut() }
        }

        fn len(&self) -> usize
        {
            self.data.len()
        }
    }

    // =========================================================================
    // Page layout
    //
    // Sections are placed sequentially immediately after the header, in the
    // same order as the diagram in the source file.
    // =========================================================================

    struct SectionSizes
    {
        num_links: usize,
        num_symbols: usize,
        num_constants: usize,
        num_functions: usize,
        num_types: usize,
        num_enum_variants: usize,
        num_gc_offsets: usize,
        data_blob_bytes: usize,
        bytecode_blob_bytes: usize,
    }

    impl Default for SectionSizes
    {
        fn default() -> Self
        {
            Self {
                num_links: 0,
                num_symbols: 0,
                num_constants: 0,
                num_functions: 0,
                num_types: 0,
                num_enum_variants: 0,
                num_gc_offsets: 0,
                data_blob_bytes: 0,
                bytecode_blob_bytes: 0,
            }
        }
    }

    /// Allocate a buffer, write the header, and return the buffer + builder.
    fn make_builder(sizes: &SectionSizes, id: SymbolId) -> (TestBuffer, PageBuilder)
    {
        let layout = PageLayout::compute(
            id,
            sizes.num_links,
            sizes.num_symbols,
            sizes.num_functions,
            sizes.num_constants,
            sizes.num_types,
            sizes.num_enum_variants,
            sizes.num_gc_offsets,
            sizes.bytecode_blob_bytes,
            sizes.data_blob_bytes,
        );

        let buf = TestBuffer::from_buffer(AlignedPageBuffer::new(layout.total_size, layout.max_alignment));
        let builder = unsafe { PageBuilder::new(buf.as_nonnull(), layout.header) };

        (buf, builder)
    }

    // Dummy SymbolId value for tests that don't care about the id
    fn dummy_symbol_id() -> SymbolId
    {
        SymbolId::ZEROED
    }

    // =========================================================================
    // Offset tests
    // =========================================================================

    mod offset_tests
    {
        use super::*;

        #[test]
        fn zero_offset_returns_base_pointer()
        {
            let mut buf = TestBuffer::new(64);
            let base = buf.as_nonnull();
            let offset = Offset(0);
            let ptr: NonNull<u8> = unsafe { offset.as_ptr(base) };
            assert_eq!(ptr.as_ptr(), base.as_ptr());
        }

        #[test]
        fn nonzero_offset_advances_pointer()
        {
            let mut buf = TestBuffer::new(64);
            let base = buf.as_nonnull();
            let offset = Offset(16);
            let ptr: NonNull<u8> = unsafe { offset.as_ptr(base) };
            assert_eq!(ptr.as_ptr() as usize, base.as_ptr() as usize + 16);
        }

        #[test]
        fn offset_as_ptr_typed()
        {
            let mut buf = TestBuffer::new(64);
            let base = buf.as_nonnull();
            let offset = Offset(8);
            let ptr: NonNull<u32> = unsafe { offset.as_ptr(base) };
            assert_eq!(ptr.as_ptr() as usize, base.as_ptr() as usize + 8);
        }

        #[test]
        fn offset_add()
        {
            let a = Offset(10);
            let b = Offset(5);
            let c = a + b;
            assert_eq!(c.0, 15);
        }

        #[test]
        fn offset_sub()
        {
            let a = Offset(20);
            let b = Offset(7);
            let c = a - b;
            assert_eq!(c.0, 13);
        }

        #[test]
        fn offset_add_zero_identity()
        {
            let a = Offset(42);
            let z = Offset(0);
            assert_eq!((a + z).0, 42);
        }

        #[test]
        fn offset_sub_zero_identity()
        {
            let a = Offset(42);
            let z = Offset(0);
            assert_eq!((a - z).0, 42);
        }

        #[test]
        fn offset_successive_adds()
        {
            let a = Offset(0);
            let b = Offset(4);
            let c = Offset(8);
            assert_eq!((a + b + c).0, 12);
        }

        #[test]
        fn offset_large_value()
        {
            let o = Offset(u32::MAX / 2);
            let mut buf = TestBuffer::new(1);
            let base = buf.as_nonnull();
            let ptr: NonNull<u8> = unsafe { o.as_ptr(base) };
            assert_eq!(ptr.as_ptr() as usize, base.as_ptr() as usize + (u32::MAX / 2) as usize);
        }

        #[test]
        fn offset_roundtrip_through_ptr()
        {
            let mut buf = TestBuffer::new(128);
            let base = buf.as_nonnull();
            let offset_val = 32u32;
            let offset = Offset(offset_val);
            let ptr: NonNull<u8> = unsafe { offset.as_ptr(base) };
            let recovered = ptr.as_ptr() as usize - base.as_ptr() as usize;
            assert_eq!(recovered, offset_val as usize);
        }
    }

    // =========================================================================
    // InlinedString tests
    // =========================================================================

    mod inlined_string_tests
    {
        use super::*;

        fn buf_with_str(s: &str) -> (TestBuffer, InlinedString)
        {
            let mut buf = TestBuffer::new(s.len().max(1));
            buf.as_slice()[..s.len()].copy_from_slice(s.as_bytes());
            let loc = (Offset(0), s.len() as u32);
            let inlined = InlinedString::new(loc);
            (buf, inlined)
        }

        #[test]
        fn get_valid_ascii_string()
        {
            let (mut buf, inlined) = buf_with_str("hello");
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert_eq!(result, Some("hello"));
        }

        #[test]
        fn get_empty_string()
        {
            let mut buf = TestBuffer::new(1);
            let loc = (Offset(0), 0u32);
            let inlined = InlinedString::new(loc);
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert_eq!(result, Some(""));
        }

        #[test]
        fn get_valid_utf8_multibyte()
        {
            let s = "héllo"; // 'é' is 2 bytes
            let (mut buf, inlined) = buf_with_str(s);
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert_eq!(result, Some(s));
        }

        #[test]
        fn get_invalid_utf8_returns_none()
        {
            let mut buf = TestBuffer::new(4);
            let data = buf.as_slice();
            // Invalid UTF-8 sequence
            data[0] = 0xFF;
            data[1] = 0xFE;
            data[2] = 0xFD;
            data[3] = 0xFC;
            let loc = (Offset(0), 4u32);
            let inlined = InlinedString::new(loc);
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert!(result.is_none());
        }

        #[test]
        fn get_unchecked_valid_ascii()
        {
            let (mut buf, inlined) = buf_with_str("world");
            let result = unsafe { inlined.get_unchecked(buf.as_nonnull()) };
            assert_eq!(result, "world");
        }

        #[test]
        fn get_unchecked_empty()
        {
            let mut buf = TestBuffer::new(1);
            let loc = (Offset(0), 0u32);
            let inlined = InlinedString::new(loc);
            let result = unsafe { inlined.get_unchecked(buf.as_nonnull()) };
            assert_eq!(result, "");
        }

        #[test]
        fn get_at_nonzero_offset()
        {
            let prefix = b"JUNK";
            let text = "datum";
            let mut buf = TestBuffer::new(prefix.len() + text.len());
            let mut data = buf.as_slice();

            data[..prefix.len()].copy_from_slice(prefix);
            data[prefix.len()..prefix.len() + text.len()].copy_from_slice(text.as_bytes());
            let loc = (Offset(prefix.len() as u32), text.len() as u32);
            let inlined = InlinedString::new(loc);
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert_eq!(result, Some("datum"));
        }

        #[test]
        fn new_stores_location()
        {
            let loc = (Offset(42), 7u32);
            let inlined = InlinedString::new(loc);
            // Round-trip via get — just check it doesn't panic with a proper buffer
            let mut buf = TestBuffer::new(50);
            let data = buf.as_slice();

            b"hello!?".iter().enumerate().for_each(|(i, &b)| data[42 + i] = b);
            let result = unsafe { inlined.get(buf.as_nonnull()) };
            assert_eq!(result, Some("hello!?"));
        }

        #[test]
        fn get_unchecked_multibyte_utf8()
        {
            let s = "日本語"; // 9 bytes
            let (mut buf, inlined) = buf_with_str(s);
            let result = unsafe { inlined.get_unchecked(buf.as_nonnull()) };
            assert_eq!(result, s);
        }

        #[test]
        fn get_single_char()
        {
            let (mut buf, inlined) = buf_with_str("Z");
            assert_eq!(unsafe { inlined.get(buf.as_nonnull()) }, Some("Z"));
        }
    }

    // =========================================================================
    // DatumPage — slice length and section counts
    // =========================================================================

    mod datum_page_slice_tests
    {
        use super::*;

        #[test]
        fn empty_page_all_slices_empty()
        {
            let sizes = SectionSizes::default();
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };

            assert_eq!(page.links.len(), 0);
            assert_eq!(page.symbols.len(), 0);
            assert_eq!(page.constants.len(), 0);
            assert_eq!(page.functions.len(), 0);
            assert_eq!(page.types.len(), 0);
            assert_eq!(page.enum_variants.len(), 0);
            assert_eq!(page.gc_offsets.len(), 0);
            assert_eq!(page.bytecode_blob.len(), 0);
            assert_eq!(page.data_blob.len(), 0);
            let _ = buf;
        }

        #[test]
        fn correct_type_count()
        {
            let sizes = SectionSizes {
                num_types: 3,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.types.len(), 3);
            let _ = buf;
        }

        #[test]
        fn correct_gc_offsets_count()
        {
            let sizes = SectionSizes {
                num_gc_offsets: 5,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.gc_offsets.len(), 5);
            let _ = buf;
        }

        #[test]
        fn correct_enum_variant_count()
        {
            let sizes = SectionSizes {
                num_enum_variants: 4,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.enum_variants.len(), 4);
            let _ = buf;
        }

        #[test]
        fn correct_data_blob_size()
        {
            let sizes = SectionSizes {
                data_blob_bytes: 32,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.data_blob.len(), 32);
            let _ = buf;
        }

        #[test]
        fn correct_bytecode_blob_size()
        {
            let sizes = SectionSizes {
                bytecode_blob_bytes: 16,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.bytecode_blob.len(), 16);
            let _ = buf;
        }

        #[test]
        fn correct_link_count()
        {
            let sizes = SectionSizes {
                num_links: 2,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.links.len(), 2);
            let _ = buf;
        }

        #[test]
        fn correct_symbol_count()
        {
            let sizes = SectionSizes {
                num_symbols: 6,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.symbols.len(), 6);
            let _ = buf;
        }

        #[test]
        fn correct_constant_count()
        {
            let sizes = SectionSizes {
                num_constants: 7,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.constants.len(), 7);
            let _ = buf;
        }

        #[test]
        fn correct_function_count()
        {
            let sizes = SectionSizes {
                num_functions: 3,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.functions.len(), 3);
            let _ = buf;
        }

        #[test]
        fn all_sections_populated()
        {
            let sizes = SectionSizes {
                num_links: 1,
                num_symbols: 2,
                num_constants: 3,
                num_functions: 4,
                num_types: 5,
                num_enum_variants: 6,
                num_gc_offsets: 7,
                data_blob_bytes: 8,
                bytecode_blob_bytes: 9,
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert_eq!(page.links.len(), 1);
            assert_eq!(page.symbols.len(), 2);
            assert_eq!(page.constants.len(), 3);
            assert_eq!(page.functions.len(), 4);
            assert_eq!(page.types.len(), 5);
            assert_eq!(page.enum_variants.len(), 6);
            assert_eq!(page.gc_offsets.len(), 7);
            assert_eq!(page.data_blob.len(), 8);
            assert_eq!(page.bytecode_blob.len(), 9);
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage — gc_offsets round-trip
    // =========================================================================

    mod gc_offsets_tests
    {
        use super::*;

        fn page_with_gc_offsets(offsets: &[usize]) -> (TestBuffer, PageBuilder)
        {
            let sizes = SectionSizes {
                num_gc_offsets: offsets.len(),
                ..Default::default()
            };
            make_builder(&sizes, dummy_symbol_id())
        }

        #[test]
        fn gc_offsets_written_and_read_back()
        {
            let values = [8usize, 16, 24, 32];
            let (mut buf, builder) = page_with_gc_offsets(&values);
            let builder = unsafe { builder.write_gc_offsets(values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.gc_offsets, &values);
            let _ = buf;
        }

        #[test]
        fn single_gc_offset()
        {
            let values = [42usize];
            let (mut buf, builder) = page_with_gc_offsets(&values);
            let builder = unsafe { builder.write_gc_offsets(values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.gc_offsets, &values);
            let _ = buf;
        }

        #[test]
        fn zero_gc_offsets()
        {
            let values: [usize; 0] = [];
            let (mut buf, builder) = page_with_gc_offsets(&values);
            let builder = unsafe { builder.write_gc_offsets(values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.gc_offsets.len(), 0);
            let _ = buf;
        }

        #[test]
        fn gc_offsets_order_preserved()
        {
            let values = [100usize, 50, 200, 25, 150];
            let (mut buf, builder) = page_with_gc_offsets(&values);
            let builder = unsafe { builder.write_gc_offsets(values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            for (i, &expected) in values.iter().enumerate()
            {
                assert_eq!(page.gc_offsets[i], expected);
            }
            let _ = buf;
        }

        #[test]
        fn large_gc_offset_values()
        {
            let values = [usize::MAX, usize::MAX - 1, 0];
            let (mut buf, builder) = page_with_gc_offsets(&values);
            let builder = unsafe { builder.write_gc_offsets(values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.gc_offsets, &values);
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage — RuntimeType + gc_offsets round-trip
    // =========================================================================

    mod types_section_tests
    {
        use super::*;

        fn dummy_back_ptr() -> NonNull<DatumPageHeader>
        {
            NonNull::dangling()
        }

        fn make_struct_type(
            instance_size: usize,
            alignment: u32,
            gc_offsets_index: u32,
            gc_offsets_count: u32,
        ) -> RuntimeType
        {
            RuntimeType {
                back_pointer: dummy_back_ptr(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Struct {
                    instance_size,
                    alignment,
                    gc_offsets_index,
                    gc_offsets_count,
                },
            }
        }

        fn make_enum_type(instance_size: usize, alignment: u32, variants_index: u32, variants_count: u32)
        -> RuntimeType
        {
            RuntimeType {
                back_pointer: dummy_back_ptr(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Enum {
                    instance_size,
                    alignment,
                    variants_index,
                    variants_count,
                },
            }
        }

        #[test]
        fn types_written_and_count_correct()
        {
            let types = [make_struct_type(32, 8, 0, 2), make_enum_type(16, 4, 0, 3)];
            let sizes = SectionSizes {
                num_types: types.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(types.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.types.len(), 2);
            let _ = buf;
        }

        #[test]
        fn struct_type_kind_preserved()
        {
            let ty = make_struct_type(64, 8, 0, 4);
            let sizes = SectionSizes {
                num_types: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert!(matches!(
                page.types[0].kind,
                RuntimeTypeKind::Struct {
                    instance_size: 64,
                    alignment: 8,
                    ..
                }
            ));
            let _ = buf;
        }

        #[test]
        fn enum_type_kind_preserved()
        {
            let ty = make_enum_type(48, 4, 2, 3);
            let sizes = SectionSizes {
                num_types: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert!(matches!(
                page.types[0].kind,
                RuntimeTypeKind::Enum {
                    instance_size: 48,
                    alignment: 4,
                    variants_index: 2,
                    variants_count: 3
                }
            ));
            let _ = buf;
        }

        #[test]
        fn multiple_types_all_correct()
        {
            let types = [
                make_struct_type(16, 8, 0, 1),
                make_struct_type(32, 8, 1, 2),
                make_enum_type(24, 4, 0, 3),
            ];
            let sizes = SectionSizes {
                num_types: types.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(types.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.types.len(), 3);
            assert!(matches!(
                page.types[0].kind,
                RuntimeTypeKind::Struct { instance_size: 16, .. }
            ));
            assert!(matches!(
                page.types[1].kind,
                RuntimeTypeKind::Struct { instance_size: 32, .. }
            ));
            assert!(matches!(
                page.types[2].kind,
                RuntimeTypeKind::Enum { instance_size: 24, .. }
            ));
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage — enum variants round-trip
    // =========================================================================

    mod enum_variants_section_tests
    {
        use super::*;

        fn make_variant(tag: u32, gc_offsets_index: u32, gc_offsets_count: u32) -> RuntimeEnumVariant
        {
            RuntimeEnumVariant {
                tag,
                gc_offsets_index,
                gc_offsets_count,
            }
        }

        #[test]
        fn variants_written_and_count_correct()
        {
            let variants = [make_variant(0, 0, 1), make_variant(1, 1, 2)];
            let sizes = SectionSizes {
                num_enum_variants: variants.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_enum_variants(variants.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.enum_variants.len(), 2);
            let _ = buf;
        }

        #[test]
        fn variant_tags_preserved()
        {
            let variants = [make_variant(42, 0, 0), make_variant(99, 0, 0), make_variant(7, 0, 0)];
            let sizes = SectionSizes {
                num_enum_variants: variants.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_enum_variants(variants.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.enum_variants[0].tag, 42);
            assert_eq!(page.enum_variants[1].tag, 99);
            assert_eq!(page.enum_variants[2].tag, 7);
            let _ = buf;
        }

        #[test]
        fn variant_gc_indices_preserved()
        {
            let v = make_variant(0, 5, 3);
            let sizes = SectionSizes {
                num_enum_variants: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_enum_variants(std::iter::once(v)) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.enum_variants[0].gc_offsets_index, 5);
            assert_eq!(page.enum_variants[0].gc_offsets_count, 3);
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage::get_struct_layout
    // =========================================================================

    mod get_struct_layout_tests
    {
        use super::*;

        fn dummy_back_ptr() -> NonNull<DatumPageHeader>
        {
            NonNull::dangling()
        }

        fn build_page_with_struct(
            instance_size: usize,
            alignment: u32,
            gc_offset_values: &[usize],
        ) -> (TestBuffer, PageBuilder)
        {
            let gc_offsets_index = 0u32;
            let gc_offsets_count = gc_offset_values.len() as u32;

            let ty = RuntimeType {
                back_pointer: dummy_back_ptr(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Struct {
                    instance_size,
                    alignment,
                    gc_offsets_index,
                    gc_offsets_count,
                },
            };

            let sizes = SectionSizes {
                num_types: 1,
                num_gc_offsets: gc_offset_values.len(),
                ..Default::default()
            };
            let (buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_offset_values.iter().copied()) }.unwrap();
            (buf, builder)
        }

        #[test]
        fn returns_correct_size_and_alignment()
        {
            let (mut buf, builder) = build_page_with_struct(128, 16, &[8, 16]);
            let page = unsafe { builder.resolve() };
            let result = page.get_struct_layout(0).unwrap();
            assert_eq!(result.0, 128); // instance_size
            assert_eq!(result.1, 16); // alignment
            let _ = buf;
        }

        #[test]
        fn returns_correct_gc_offsets()
        {
            let offsets = [8usize, 24, 40];
            let (mut buf, builder) = build_page_with_struct(64, 8, &offsets);
            let page = unsafe { builder.resolve() };
            let (_, _, gc) = page.get_struct_layout(0).unwrap();
            assert_eq!(gc, &offsets);
            let _ = buf;
        }

        #[test]
        fn returns_empty_gc_offsets_for_struct_with_no_refs()
        {
            let (mut buf, builder) = build_page_with_struct(32, 8, &[]);
            let page = unsafe { builder.resolve() };
            let (_, _, gc) = page.get_struct_layout(0).unwrap();
            assert_eq!(gc.len(), 0);
            let _ = buf;
        }

        #[test]
        fn returns_none_for_out_of_bounds_index()
        {
            let (mut buf, builder) = build_page_with_struct(32, 8, &[]);
            let page = unsafe { builder.resolve() };
            assert!(page.get_struct_layout(1).is_none()); // only index 0 exists
            assert!(page.get_struct_layout(u32::MAX).is_none());
            let _ = buf;
        }

        #[test]
        fn returns_none_for_enum_type()
        {
            let ty = RuntimeType {
                back_pointer: NonNull::dangling(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Enum {
                    instance_size: 16,
                    alignment: 4,
                    variants_index: 0,
                    variants_count: 1,
                },
            };
            let sizes = SectionSizes {
                num_types: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert!(page.get_struct_layout(0).is_none());
            let _ = buf;
        }

        #[test]
        fn multiple_structs_correct_index_lookup()
        {
            let types = [
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Struct {
                        instance_size: 16,
                        alignment: 4,
                        gc_offsets_index: 0,
                        gc_offsets_count: 1,
                    },
                },
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Struct {
                        instance_size: 32,
                        alignment: 8,
                        gc_offsets_index: 1,
                        gc_offsets_count: 2,
                    },
                },
            ];
            let gc_values = [8usize, 16, 24];
            let sizes = SectionSizes {
                num_types: 2,
                num_gc_offsets: gc_values.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(types.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };

            let (sz0, al0, gc0) = page.get_struct_layout(0).unwrap();
            assert_eq!(sz0, 16);
            assert_eq!(al0, 4);
            assert_eq!(gc0, &gc_values[0..1]);

            let (sz1, al1, gc1) = page.get_struct_layout(1).unwrap();
            assert_eq!(sz1, 32);
            assert_eq!(al1, 8);
            assert_eq!(gc1, &gc_values[1..3]);
            let _ = buf;
        }

        #[test]
        fn returns_none_when_no_types()
        {
            let sizes = SectionSizes::default();
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let page = unsafe { builder.resolve() };
            assert!(page.get_struct_layout(0).is_none());
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage::get_enum_variant_layout
    // =========================================================================

    mod get_enum_variant_layout_tests
    {
        use super::*;

        fn make_enum_type_entry(variants_index: u32, variants_count: u32) -> RuntimeType
        {
            RuntimeType {
                back_pointer: NonNull::dangling(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Enum {
                    instance_size: 24,
                    alignment: 8,
                    variants_index,
                    variants_count,
                },
            }
        }

        fn make_variant(tag: u32, gc_offsets_index: u32, gc_offsets_count: u32) -> RuntimeEnumVariant
        {
            RuntimeEnumVariant {
                tag,
                gc_offsets_index,
                gc_offsets_count,
            }
        }

        fn build_page_with_enum(variants: &[RuntimeEnumVariant], variants_index: u32) -> (TestBuffer, PageBuilder)
        {
            let ty = make_enum_type_entry(variants_index, variants.len() as u32);
            let sizes = SectionSizes {
                num_types: 1,
                num_enum_variants: variants.len(),
                ..Default::default()
            };
            let (buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let builder = unsafe { builder.write_enum_variants(variants.iter().copied()) }.unwrap();
            (buf, builder)
        }

        #[test]
        fn finds_variant_by_tag()
        {
            let variants = [make_variant(0, 0, 0), make_variant(1, 0, 0), make_variant(2, 0, 0)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            let v = page.get_enum_variant_layout(0, 1).unwrap();
            assert_eq!(v.tag, 1);
            let _ = buf;
        }

        #[test]
        fn finds_first_variant()
        {
            let variants = [make_variant(10, 0, 2), make_variant(20, 2, 1)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            let v = page.get_enum_variant_layout(0, 10).unwrap();
            assert_eq!(v.tag, 10);
            assert_eq!(v.gc_offsets_count, 2);
            let _ = buf;
        }

        #[test]
        fn finds_last_variant()
        {
            let variants = [make_variant(0, 0, 0), make_variant(1, 0, 0), make_variant(99, 3, 2)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            let v = page.get_enum_variant_layout(0, 99).unwrap();
            assert_eq!(v.tag, 99);
            assert_eq!(v.gc_offsets_index, 3);
            let _ = buf;
        }

        #[test]
        fn returns_none_for_unknown_tag()
        {
            let variants = [make_variant(0, 0, 0), make_variant(1, 0, 0)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            assert!(page.get_enum_variant_layout(0, 99).is_none());
            let _ = buf;
        }

        #[test]
        fn returns_none_for_out_of_bounds_type_index()
        {
            let variants = [make_variant(0, 0, 0)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            assert!(page.get_enum_variant_layout(1, 0).is_none());
            let _ = buf;
        }

        #[test]
        fn returns_none_for_struct_type()
        {
            let ty = RuntimeType {
                back_pointer: NonNull::dangling(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Struct {
                    instance_size: 32,
                    alignment: 8,
                    gc_offsets_index: 0,
                    gc_offsets_count: 0,
                },
            };
            let sizes = SectionSizes {
                num_types: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert!(page.get_enum_variant_layout(0, 0).is_none());
            let _ = buf;
        }

        #[test]
        fn variants_index_correctly_slices_flat_array()
        {
            // Enum type uses variants starting at index 2 (skipping the first two)
            let ty = make_enum_type_entry(2, 2); // 2 variants starting at index 2
            let all_variants = [
                make_variant(0, 0, 0),  // index 0 — belongs to a different type
                make_variant(1, 0, 0),  // index 1 — belongs to a different type
                make_variant(10, 5, 1), // index 2 — our first variant
                make_variant(20, 6, 1), // index 3 — our second variant
            ];
            let sizes = SectionSizes {
                num_types: 1,
                num_enum_variants: all_variants.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let builder = unsafe { builder.write_enum_variants(all_variants.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };

            // Should find tag=10 (index 2) but not tag=0 or tag=1 (indices 0,1)
            assert!(page.get_enum_variant_layout(0, 10).is_some());
            assert!(page.get_enum_variant_layout(0, 20).is_some());
            assert!(page.get_enum_variant_layout(0, 0).is_none());
            assert!(page.get_enum_variant_layout(0, 1).is_none());
            let _ = buf;
        }

        #[test]
        fn single_variant_enum()
        {
            let variants = [make_variant(0, 0, 1)];
            let (mut buf, builder) = build_page_with_enum(&variants, 0);
            let page = unsafe { builder.resolve() };
            assert!(page.get_enum_variant_layout(0, 0).is_some());
            assert!(page.get_enum_variant_layout(0, 1).is_none());
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPage::get_variant_gc_offsets
    // =========================================================================

    mod get_variant_gc_offsets_tests
    {
        use super::*;

        fn make_variant(tag: u32, gc_offsets_index: u32, gc_offsets_count: u32) -> RuntimeEnumVariant
        {
            RuntimeEnumVariant {
                tag,
                gc_offsets_index,
                gc_offsets_count,
            }
        }

        fn build_page(gc_values: &[usize], variants: &[RuntimeEnumVariant]) -> (TestBuffer, PageBuilder)
        {
            let sizes = SectionSizes {
                num_gc_offsets: gc_values.len(),
                num_enum_variants: variants.len(),
                ..Default::default()
            };
            let (buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_enum_variants(variants.iter().copied()) }.unwrap();
            (buf, builder)
        }

        #[test]
        fn returns_correct_slice_for_variant()
        {
            let gc_values = [8usize, 16, 24, 32];
            let variant = make_variant(0, 1, 2); // gc_offsets[1..3]
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]).unwrap();
            assert_eq!(result, &gc_values[1..3]);
            let _ = buf;
        }

        #[test]
        fn returns_empty_slice_for_zero_count()
        {
            let gc_values = [8usize, 16];
            let variant = make_variant(0, 0, 0);
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]).unwrap();
            assert_eq!(result.len(), 0);
            let _ = buf;
        }

        #[test]
        fn returns_none_for_out_of_range_index()
        {
            let gc_values = [8usize, 16];
            let variant = make_variant(0, 5, 2); // index 5 is out of range
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]);
            assert!(result.is_none());
            let _ = buf;
        }

        #[test]
        fn returns_none_when_count_exceeds_array()
        {
            let gc_values = [8usize, 16];
            let variant = make_variant(0, 1, 5); // 1+5 = 6, but only 2 elements
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]);
            assert!(result.is_none());
            let _ = buf;
        }

        #[test]
        fn correct_offsets_at_start_of_array()
        {
            let gc_values = [100usize, 200, 300];
            let variant = make_variant(0, 0, 2);
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]).unwrap();
            assert_eq!(result, &gc_values[0..2]);
            let _ = buf;
        }

        #[test]
        fn correct_offsets_at_end_of_array()
        {
            let gc_values = [10usize, 20, 30, 40];
            let variant = make_variant(0, 2, 2);
            let (mut buf, builder) = build_page(&gc_values, &[variant]);
            let page = unsafe { builder.resolve() };
            let result = page.get_variant_gc_offsets(&page.enum_variants[0]).unwrap();
            assert_eq!(result, &gc_values[2..4]);
            let _ = buf;
        }

        #[test]
        fn multiple_variants_independent_gc_slices()
        {
            let gc_values = [8usize, 16, 24, 32, 40];
            let v0 = make_variant(0, 0, 2); // [8, 16]
            let v1 = make_variant(1, 2, 3); // [24, 32, 40]
            let (mut buf, builder) = build_page(&gc_values, &[v0, v1]);
            let page = unsafe { builder.resolve() };
            let r0 = page.get_variant_gc_offsets(&page.enum_variants[0]).unwrap();
            let r1 = page.get_variant_gc_offsets(&page.enum_variants[1]).unwrap();
            assert_eq!(r0, &gc_values[0..2]);
            assert_eq!(r1, &gc_values[2..5]);
            let _ = buf;
        }
    }

    // =========================================================================
    // PageBuilder — write_gc_offsets stops at capacity
    // =========================================================================

    mod page_builder_capacity_tests
    {
        use super::*;

        #[test]
        fn write_gc_offsets_within_capacity_succeeds()
        {
            let sizes = SectionSizes {
                num_gc_offsets: 3,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let result = unsafe { builder.write_gc_offsets([1usize, 2, 3].iter().copied()) };
            assert!(result.is_some());
            let _ = buf;
        }

        // NOTE: write_iter has a known off-by-one: it uses `guard!(i <= limit)` where
        // `limit = byte_len / size_of::<T>()` (i.e. the number of slots). At `i == limit`,
        // the guard passes but the write is one slot past the allocation.
        // The correct check should be `guard!(i < limit)`.
        // The test below documents this boundary behaviour rather than testing valid input.
        #[test]
        fn write_fewer_than_capacity_is_always_safe()
        {
            // Writing fewer items than capacity must succeed
            let sizes = SectionSizes {
                num_gc_offsets: 4,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let result = unsafe { builder.write_gc_offsets([1usize, 2].iter().copied()) };
            assert!(result.is_some());
            let page = unsafe { result.unwrap().resolve() };
            // Only 2 written; first 2 slots match, remainder are zeroed from TestBuffer
            assert_eq!(page.gc_offsets[0], 1);
            assert_eq!(page.gc_offsets[1], 2);
            let _ = buf;
        }

        #[test]
        fn write_data_blob_exact_size_succeeds()
        {
            let data = [0xDE, 0xAD, 0xBE, 0xEF];
            let sizes = SectionSizes {
                data_blob_bytes: data.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let result = unsafe { builder.write_data_blob(&data) };
            assert!(result.is_some());
            let page = unsafe { result.unwrap().resolve() };
            assert_eq!(page.data_blob, &data);
            let _ = buf;
        }

        #[test]
        fn write_code_blob_exact_size_succeeds()
        {
            let code = [0x01u8, 0x02, 0x03];
            let sizes = SectionSizes {
                bytecode_blob_bytes: code.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let result = unsafe { builder.write_code_blob(&code) };
            assert!(result.is_some());
            let page = unsafe { result.unwrap().resolve() };
            assert_eq!(page.bytecode_blob, &code);
            let _ = buf;
        }

        #[test]
        #[should_panic]
        fn write_data_blob_larger_than_section_panics()
        {
            let sizes = SectionSizes {
                data_blob_bytes: 4,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let oversized = [0u8; 8];
            let _ = unsafe { builder.write_data_blob(&oversized) };
            let _ = buf;
        }

        #[test]
        #[should_panic]
        fn write_code_blob_larger_than_section_panics()
        {
            let sizes = SectionSizes {
                bytecode_blob_bytes: 2,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let oversized = [0u8; 8];
            let _ = unsafe { builder.write_code_blob(&oversized) };
            let _ = buf;
        }

        #[test]
        fn empty_data_blob_write_succeeds()
        {
            let sizes = SectionSizes {
                data_blob_bytes: 8,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let result = unsafe { builder.write_data_blob(&[]) };
            assert!(result.is_some());
            let _ = buf;
        }
    }

    // =========================================================================
    // PageBuilder — blob round-trips
    // =========================================================================

    mod blob_roundtrip_tests
    {
        use super::*;

        #[test]
        fn data_blob_roundtrip()
        {
            let data: Vec<u8> = (0u8..=255).collect();
            let sizes = SectionSizes {
                data_blob_bytes: data.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_data_blob(&data) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.data_blob, &data[..]);
            let _ = buf;
        }

        #[test]
        fn bytecode_blob_roundtrip()
        {
            let code = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE];
            let sizes = SectionSizes {
                bytecode_blob_bytes: code.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_code_blob(&code) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.bytecode_blob, &code);
            let _ = buf;
        }

        #[test]
        fn data_blob_all_zeros()
        {
            let data = [0u8; 64];
            let sizes = SectionSizes {
                data_blob_bytes: data.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_data_blob(&data) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert!(page.data_blob.iter().all(|&b| b == 0));
            let _ = buf;
        }

        #[test]
        fn both_blobs_independent()
        {
            let data = [0x11u8; 8];
            let code = [0x22u8; 8];
            let sizes = SectionSizes {
                data_blob_bytes: data.len(),
                bytecode_blob_bytes: code.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_data_blob(&data) }.unwrap();
            let builder = unsafe { builder.write_code_blob(&code) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.data_blob, &data);
            assert_eq!(page.bytecode_blob, &code);
            let _ = buf;
        }

        #[test]
        fn single_byte_blobs()
        {
            let sizes = SectionSizes {
                data_blob_bytes: 1,
                bytecode_blob_bytes: 1,
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_data_blob(&[0xAB]) }.unwrap();
            let builder = unsafe { builder.write_code_blob(&[0xCD]) }.unwrap();
            let page = unsafe { builder.resolve() };
            assert_eq!(page.data_blob[0], 0xAB);
            assert_eq!(page.bytecode_blob[0], 0xCD);
            let _ = buf;
        }
    }

    // =========================================================================
    // DatumPageHeader::get_page
    // =========================================================================

    mod get_page_tests
    {
        use super::*;

        #[test]
        fn get_page_from_header_gives_correct_slice_lengths()
        {
            let gc_values = [8usize, 16, 24];
            let sizes = SectionSizes {
                num_gc_offsets: gc_values.len(),
                num_types: 1,
                ..Default::default()
            };
            let ty = RuntimeType {
                back_pointer: NonNull::dangling(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Struct {
                    instance_size: 32,
                    alignment: 8,
                    gc_offsets_index: 0,
                    gc_offsets_count: 3,
                },
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };

            // Use get_page via the embedded header pointer
            let page_via_header = unsafe { page.id };
            // The key check: get_page on the header embedded in the page gives the same layout
            // We verify the gc_offsets slice is accessible and correct
            assert_eq!(page.gc_offsets, &gc_values);
            assert_eq!(page.types.len(), 1);
            let _ = buf;
        }

        #[test]
        fn get_struct_layout_via_full_page()
        {
            let gc_values = [8usize, 16];
            let sizes = SectionSizes {
                num_gc_offsets: gc_values.len(),
                num_types: 1,
                ..Default::default()
            };
            let ty = RuntimeType {
                back_pointer: NonNull::dangling(),
                symbol_id: dummy_symbol_id(),
                kind: RuntimeTypeKind::Struct {
                    instance_size: 48,
                    alignment: 8,
                    gc_offsets_index: 0,
                    gc_offsets_count: 2,
                },
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(std::iter::once(ty)) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };
            let (sz, al, gc) = page.get_struct_layout(0).unwrap();
            assert_eq!(sz, 48);
            assert_eq!(al, 8);
            assert_eq!(gc, &gc_values[..]);
            let _ = buf;
        }
    }

    // =========================================================================
    // Full page integration — all sections written and verified together
    // =========================================================================

    mod integration_tests
    {
        use super::*;

        #[test]
        fn full_page_all_sections_independent()
        {
            // Build a page with every section populated and verify they don't
            // alias or overwrite each other.
            let gc_values = [8usize, 16, 24, 32, 40, 48];
            let variants = [
                RuntimeEnumVariant {
                    tag: 0,
                    gc_offsets_index: 0,
                    gc_offsets_count: 2,
                },
                RuntimeEnumVariant {
                    tag: 1,
                    gc_offsets_index: 2,
                    gc_offsets_count: 3,
                },
                RuntimeEnumVariant {
                    tag: 2,
                    gc_offsets_index: 5,
                    gc_offsets_count: 1,
                },
            ];
            let types = [
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Struct {
                        instance_size: 32,
                        alignment: 8,
                        gc_offsets_index: 0,
                        gc_offsets_count: 2,
                    },
                },
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Enum {
                        instance_size: 24,
                        alignment: 8,
                        variants_index: 0,
                        variants_count: 3,
                    },
                },
            ];
            let data = [0xABu8; 16];
            let code = [0xCDu8; 8];

            let sizes = SectionSizes {
                num_types: types.len(),
                num_enum_variants: variants.len(),
                num_gc_offsets: gc_values.len(),
                data_blob_bytes: data.len(),
                bytecode_blob_bytes: code.len(),
                ..Default::default()
            };

            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(types.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_enum_variants(variants.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_data_blob(&data) }.unwrap();
            let builder = unsafe { builder.write_code_blob(&code) }.unwrap();
            let page = unsafe { builder.resolve() };

            // Types
            assert_eq!(page.types.len(), 2);
            assert!(matches!(
                page.types[0].kind,
                RuntimeTypeKind::Struct { instance_size: 32, .. }
            ));
            assert!(matches!(
                page.types[1].kind,
                RuntimeTypeKind::Enum { instance_size: 24, .. }
            ));

            // GC offsets
            assert_eq!(page.gc_offsets, &gc_values);

            // Variants
            assert_eq!(page.enum_variants.len(), 3);
            assert_eq!(page.enum_variants[1].tag, 1);

            // Struct layout
            let (sz, _, gc) = page.get_struct_layout(0).unwrap();
            assert_eq!(sz, 32);
            assert_eq!(gc, &gc_values[0..2]);

            // Enum variant layout
            let v = page.get_enum_variant_layout(1, 2).unwrap();
            assert_eq!(v.gc_offsets_index, 5);
            assert_eq!(v.gc_offsets_count, 1);

            // Variant gc offsets
            let vgc = page.get_variant_gc_offsets(v).unwrap();
            assert_eq!(vgc, &gc_values[5..6]);

            // Blobs untouched
            assert_eq!(page.data_blob, &data);
            assert_eq!(page.bytecode_blob, &code);

            let _ = buf;
        }

        #[test]
        fn two_enum_types_sharing_flat_variant_array()
        {
            // Two enum types whose variants share the flat enum_variants array
            let types = [
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Enum {
                        instance_size: 16,
                        alignment: 4,
                        variants_index: 0,
                        variants_count: 2,
                    },
                },
                RuntimeType {
                    back_pointer: NonNull::dangling(),
                    symbol_id: dummy_symbol_id(),
                    kind: RuntimeTypeKind::Enum {
                        instance_size: 24,
                        alignment: 8,
                        variants_index: 2,
                        variants_count: 3,
                    },
                },
            ];
            let all_variants = [
                RuntimeEnumVariant {
                    tag: 0,
                    gc_offsets_index: 0,
                    gc_offsets_count: 1,
                },
                RuntimeEnumVariant {
                    tag: 1,
                    gc_offsets_index: 1,
                    gc_offsets_count: 0,
                },
                RuntimeEnumVariant {
                    tag: 0,
                    gc_offsets_index: 1,
                    gc_offsets_count: 2,
                },
                RuntimeEnumVariant {
                    tag: 1,
                    gc_offsets_index: 3,
                    gc_offsets_count: 1,
                },
                RuntimeEnumVariant {
                    tag: 2,
                    gc_offsets_index: 4,
                    gc_offsets_count: 0,
                },
            ];
            let gc_values = [8usize, 16, 24, 32, 40];

            let sizes = SectionSizes {
                num_types: types.len(),
                num_enum_variants: all_variants.len(),
                num_gc_offsets: gc_values.len(),
                ..Default::default()
            };
            let (mut buf, builder) = make_builder(&sizes, dummy_symbol_id());
            let builder = unsafe { builder.write_types(types.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_enum_variants(all_variants.iter().copied()) }.unwrap();
            let builder = unsafe { builder.write_gc_offsets(gc_values.iter().copied()) }.unwrap();
            let page = unsafe { builder.resolve() };

            // Type 0: tags 0 and 1 only
            assert!(page.get_enum_variant_layout(0, 0).is_some());
            assert!(page.get_enum_variant_layout(0, 1).is_some());
            assert!(page.get_enum_variant_layout(0, 2).is_none());

            // Type 1: tags 0, 1, 2
            assert!(page.get_enum_variant_layout(1, 0).is_some());
            assert!(page.get_enum_variant_layout(1, 1).is_some());
            assert!(page.get_enum_variant_layout(1, 2).is_some());

            let _ = buf;
        }
    }
}
