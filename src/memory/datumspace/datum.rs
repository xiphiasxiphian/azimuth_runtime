use std::{marker::PhantomData, ptr::NonNull};

use crate::memory::datumspace::{constant_table::Constant, link_table::Link, runnable::Runnable, tables::symbol_table::Symbol};

/*  ┌─────────────────────────┐  <- base_ptr
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
    │  data blob              │  raw bytes for all constants
    ├─────────────────────────┤
    │  code blob              │  raw bytecode for all functions
    └─────────────────────────┘

 */

#[derive(Clone, Copy, Debug)]
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
pub struct InlinedString<'a>
{
    location: BlockLocation,
    _pd: PhantomData<&'a str>,
}

impl<'a> InlinedString<'a>
{
    pub fn new(location: BlockLocation) -> Self
    {
        Self { location, _pd: PhantomData }
    }

    pub unsafe fn get(&self, base: NonNull<u8>) -> Option<&'a str>
    {
        unsafe {
            let bytes: &[u8] = std::slice::from_raw_parts(self.location.0.as_ptr(base).as_ptr(), self.location.1 as usize);
            str::from_utf8(bytes).ok()
        }
    }

    pub unsafe fn get_unchecked(&self, base: NonNull<u8>) -> &'a str
    {
        unsafe {
            let bytes: &[u8] = std::slice::from_raw_parts(self.location.0.as_ptr(base).as_ptr(), self.location.1 as usize);
            str::from_utf8_unchecked(bytes)
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DatumPageHeader
{
    pub id: BlockLocation,
    pub link_table: BlockLocation,
    pub symbol_table: BlockLocation,
    pub constants: BlockLocation,
    pub functions: BlockLocation,
    pub bytecode_blob: BlockLocation,
    pub data_blob: BlockLocation,
}

impl DatumPageHeader
{

    /// Constructs a view over the Datumpage, from its header
    ///
    /// SAFETY: This is only safe is the given page header is embedded within datumspace
    /// in an _actual_ DatumPage. Otherwise, this will end up reading into nonsense memory
    pub unsafe fn get_page(&self) -> DatumPage
    {
        unsafe { DatumPage::from_base_ptr(NonNull::from_ref(self).cast()) }
    }
}

/// A typed view over a raw DatumPage block.
/// All slices point into the same contiguous allocation.
pub struct DatumPage<'a>
{
    pub id: &'a str,
    pub links: &'a [Link<'a>],
    pub symbols: &'a [Symbol<'a>],
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
        let id = unsafe {
            let bytes = std::slice::from_raw_parts(ptr.byte_add(header.id.0.0 as usize).as_ptr(), header.id.1 as usize);
            str::from_utf8_unchecked(bytes)
        };

        // link table
        let links: &'a [Link<'a>] = unsafe {
            Self::get_slice(ptr, header.link_table)
        };

        // symbol table
        let symbols: &'a [Symbol<'a>] = unsafe {
            Self::get_slice(ptr, header.symbol_table)
        };

        // bytecode and function headers
        let bytecode_blob: &'a [u8] = unsafe {
            Self::get_slice(ptr, header.bytecode_blob)
        };

        // All constants and other data
        let data_blob: &'a [u8] = unsafe {
          Self::get_slice(ptr, header.data_blob)
        };

        Self {
            id,
            links,
            symbols,
            bytecode_blob,
            data_blob,
        }
    }

    unsafe fn get_slice<T>(base: NonNull<u8>, location: BlockLocation) -> &'a [T]
    where T: Sized
    {
        unsafe {
            std::slice::from_raw_parts(base.byte_add(location.0.0 as usize).cast().as_ptr(), location.1 as usize / size_of::<T>())
        }
    }
}

pub fn align_up(offset: usize, align: usize) -> usize
{
    (offset + align - 1) & !(align - 1)
}
