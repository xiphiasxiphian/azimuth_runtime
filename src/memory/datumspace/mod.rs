pub mod types;
pub mod constant_table;
pub mod runnable;
mod datum;

use std::{alloc::Layout, collections::HashMap, ptr::NonNull};

use crate::{loader::parser::{function::{Directive, FunctionInfo}, table::{Table, TableEntry}}, memory::{allocators::{AllocatorError, general::GeneralAllocator}, datumspace::{constant_table::Constant, datum::{DatumPage, DatumPageHeader, align_up}, runnable::Runnable, types::TypeInfo}}};

/*
 +---------------------+       +-------------------------+       +-------------------------+
 |     1. File I/O     |       |   2. Transient Parsing  |       | 3. Datumspace Storage   |
 |  (Global Memory)    |       |   (Native Heap/Stack)   |       |  (Custom Allocator)     |
 +---------------------+       +-------------------------+       +-------------------------+
 |                     |       |                         |       |                         |
 |  [u8 Buffer]        |       |  Table<'file>           |       |  Datumspace<'datum>     |
 |  "05 0A 00 00 00..."| ----> |  Vec<TableEntry<'file>> | ----> |  GeneralAllocator       |
 |                     |       |    |-- Integer(42)      |       |    |-- Integer(42)      |
 |                     |       |    |-- String(&str)  ---------+ |    |-- String(&str)     |
 +---------------------+       +-------------------------+     | +-------------------------+
           |                               |                   |               ^
           | Drops when file closed        | Drops after phase |               | Bytes copied into
           v                               v                   +---------------+ allocator memory
      [Memory Freed]                 [Memory Freed]              (Data lives as long as Datumspace)
 */


const ALLOCATOR_DEPTH: usize = 8;
type DatumAllocator = GeneralAllocator<ALLOCATOR_DEPTH>;

#[derive(Clone, Copy, Debug)]
pub enum DatumspaceError
{
    LeftOverBytes,
    InvalidStructure,
    AllocationFailure,
    Duplication,
    UnexpectedDatumtype,
    ResourceDoesntExist,
}

enum DatumEntry<'a>
{
    ConstantTable(&'a [Constant<'a>]),
    Function(&'a Runnable<'a>),
    Type(),
}

pub struct Datumspace<'a>
{
    allocator: DatumAllocator,
    mapping: HashMap<&'a str, DatumEntry<'a>>
}

impl<'d> Datumspace<'d>
{
    pub fn with_capacity(min_capacity: usize) -> Result<Self, AllocatorError>
    {
        Ok(
            Self {
                allocator: DatumAllocator::with_capacity(min_capacity)?,
                mapping: HashMap::new(),
            }
        )
    }

    pub fn load_datum<'file>(
        &'d mut self,
        id: &'file str,
        table: &[TableEntry<'file>],
        functions: &'file [FunctionInfo<'file>],
    ) -> Result<DatumPage<'d>, DatumspaceError>
    where
        'd: 'file
    {
        let page_layout = Self::calculate_page_size(id, table, functions)?;
        let base: NonNull<u8> = self.allocator.raw_alloc(page_layout).ok_or(DatumspaceError::AllocationFailure)?;

        // Construct header based on know values
        let header = DatumPageHeader {
            id_len: id.len() as u32,
            constants_len: table.len() as u32,
            functions_len: functions.len() as u32,
        };

        // Write header to start of block
        unsafe { base.cast().write(header) };

        // We maintain a byte cursor for blobs (strings, directives, bytecode)
        // that starts after the fixed-size arrays and walks forward.
        let const_offset = align_up(
            size_of::<DatumPageHeader>() + id.len(),
            align_of::<Constant>(),
        );

        let fn_offset = align_up(
            const_offset + table.len() * size_of::<Constant>(),
            align_of::<Runnable>(),
        );

        // Blob cursor begins after the functions array
        let mut blob_cursor = fn_offset + functions.len() * size_of::<Runnable>();

        // Write id to start
        unsafe {
            let id_dest = base.byte_add(size_of::<DatumPageHeader>());
            std::ptr::copy_nonoverlapping(id.as_ptr(), id_dest.as_ptr(), id.len());
        };

        // Write constants
        let const_base = unsafe { base.byte_add(const_offset).cast() };

        for (i, entry) in table.iter().enumerate()
        {
            let constant: Constant<'d> = match entry {
                TableEntry::Integer(v) => Constant::Unsigned32(*v),
                TableEntry::Long(v)    => Constant::Unsigned64(*v),
                TableEntry::Float(v)   => Constant::Float32(*v),
                TableEntry::Double(v)  => Constant::Float64(*v),
                TableEntry::String(s)  => {
                    // Write blob at cursor, produce a 'd slice into the page
                    let blob_ptr = unsafe { base.byte_add(blob_cursor) };
                    unsafe {
                        blob_ptr.copy_from_nonoverlapping(NonNull::new_unchecked(s.as_ptr() as *mut _), s.len());
                    }
                    let permanent: &'d str = unsafe {
                        str::from_utf8_unchecked(
                            NonNull::slice_from_raw_parts(blob_ptr, s.len()).as_ref()
                        )
                    };
                    blob_cursor += s.len();
                    Constant::String(permanent)
                }
            };
            unsafe { const_base.add(i).write(constant) };
        }

        // Write functions
        let fn_base  = unsafe { base.byte_add(fn_offset).cast::<Runnable<'d>>() };
        let constants: &'d [Constant<'d>] = unsafe {
            std::slice::from_raw_parts(const_base.as_ptr(), table.len())
        };

        for (i, info) in functions.iter().enumerate()
        {
            // Resolve name from the constants we just wrote
            let name = match constants.get(info.name_index) {
                Some(Constant::String(s)) => *s,
                Some(_) => return Err(DatumspaceError::UnexpectedDatumtype),
                None    => return Err(DatumspaceError::ResourceDoesntExist),
            };

            // Write directives blob
            blob_cursor = align_up(blob_cursor, align_of::<Directive>());
            let dir_ptr  = unsafe { base.byte_add(blob_cursor).cast::<Directive>() };
            let dir_len  = info.directives.len();

            // TODO: Technically some directives are removed as they are the required ones
            blob_cursor += dir_len * size_of::<Directive>();

            // Write bytecode blob
            let code_ptr = unsafe { base.byte_add(blob_cursor) };
            let code_len = info.code.len();


            blob_cursor += code_len;

            // Build the Runnable directly into the page — no allocator call needed
            // since directives and bytecode now live in the page block itself.
            let runnable = unsafe {
                Runnable::from_parsed_data(
                    fn_base.add(i),
                    dir_ptr,
                    code_ptr,
                    name,
                    &info.directives,
                    info.code
                )?
            };

            // Register name -> function pointer in the flat lookup map
            if self.mapping.insert(name, DatumEntry::Function(runnable)).is_some() {
                return Err(DatumspaceError::Duplication);
            }
        }

        let page = unsafe { DatumPage::from_base_ptr(base.as_ptr() as *const _) };

        Ok(page)
    }

    pub fn get_constant(&self, id: &str, index: usize) -> Result<&'d Constant<'d>, DatumspaceError>
    {
        // The main difference here is that the id refers to the datumpage rather than a specific entry in it, as
        // every page only has one constant table.
        todo!()
    }

    pub fn get_runnable(&self, id: &str) -> Result<&'d Runnable<'d>, DatumspaceError>
    {
        match self.mapping.get(id)
        {
            Some(&DatumEntry::Function(runnable)) => Ok(runnable),
            Some(_) => Err(DatumspaceError::UnexpectedDatumtype),
            None => Err(DatumspaceError::ResourceDoesntExist),
        }
    }

    fn calculate_page_size<'file>(
        table_id:  &str,
        table:     &[TableEntry<'file>],
        functions: &[FunctionInfo<'file>],
    ) -> Result<Layout, DatumspaceError> {
        let mut size = size_of::<DatumPageHeader>();

        // id
        size = align_up(size + table_id.len(), align_of::<Constant>());

        // constants array
        size += table.len() * size_of::<Constant>();

        // string blobs inside constants
        for entry in table.iter() {
            if let TableEntry::String(s) = entry {
                size = align_up(size + s.len(), align_of::<u8>());
            }
        }

        // align up to Runnable before the functions array
        size = align_up(size, align_of::<Runnable>());
        size += functions.len() * size_of::<Runnable>();

        // directives + bytecode blobs per function
        for info in functions.iter() {
            size = align_up(size, align_of::<Directive>());
            size += info.directives.len() * size_of::<Directive>();
            size += info.code.len(); // bytecode is u8, no alignment needed
        }

        Layout::from_size_align(size, align_of::<DatumPageHeader>())
            .map_err(|_| DatumspaceError::AllocationFailure)
    }
}

#[cfg(test)]
mod datumspace_tests {
    use super::*;

    #[test]
    fn can_create()
    {
        assert!(true);
    }
}
