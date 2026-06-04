use std::{ops::Add as _, ptr::NonNull};

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
        unsafe {
            std::slice::from_raw_parts(
                base.byte_add(location.0.0 as usize).cast().as_ptr(),
                location.1 as usize / size_of::<T>(),
            )
        }
    }

    /// Get the pre-calculated GC offsets for a struct by its index
    pub fn get_struct_layout(&self, type_index: u32) -> Option<(usize, &'a [usize])>
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
            Some((instance_size, self.gc_offsets.get(start..end)?))
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
            instance_size,
            alignment,
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
        unsafe { self.write_iter(&self.base.as_ref().constants, src) }
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
        T: Copy + Sized,
    {
        let base: NonNull<T> = unsafe { loc.0.as_ptr(self.base.cast()) };
        let limit = <usize>::try_from(loc.1).ok()? / size_of::<T>();

        for (i, item) in iter.enumerate()
        {
            guard!(i <= limit);
            unsafe { base.add(i).write(item) }
        }

        Some(self)
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn tmp() {}
}
