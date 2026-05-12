pub mod datum;
pub mod runnable;
pub mod tables;
pub mod types;

use std::{
    alloc::Layout,
    collections::{HashMap, hash_map::Entry},
    marker::PhantomData,
    ptr::NonNull,
};

use itertools::{Itertools, process_results};

use crate::{
    loader::{
        SymbolId,
        parser::layout::{DataHeader, FileLayout, SymbolKind as ParsedSymbolKind},
    },
    memory::{
        allocators::{AllocatorError, general::GeneralAllocator},
        datumspace::{
            datum::{BlockLocation, DatumPage, DatumPageHeader, InlinedString, Offset, PageBuilder},
            runnable::{Function, FunctionFlags, Runnable},
            tables::{
                constant_table::{Constant, ConstantTableEntry, DataEntry},
                link_table::{self, Link},
                symbol_table::{Symbol, SymbolKind},
            },
        },
    },
};

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

type DatumResult<T, E = DatumspaceError> = Result<T, E>;

#[derive(Clone, Copy, Debug)]
pub enum DatumspaceError
{
    InvalidStructure,
    AllocationFailure,
    Duplication,
    InvalidConstantType,
    ResourceDoesntExist,
    PageNotLoaded,
}

#[derive(Debug)]
pub struct Datumspace<'a>
{
    allocator: DatumAllocator,
    mapping: HashMap<SymbolId, NonNull<u8>>,
    _pd: PhantomData<&'a mut [u8]>,
}

impl<'d> Datumspace<'d>
{
    pub fn with_capacity(min_capacity: usize) -> DatumResult<Self, AllocatorError>
    {
        Ok(Self {
            allocator: DatumAllocator::with_capacity(min_capacity)?,
            mapping: HashMap::new(),
            _pd: PhantomData,
        })
    }

    pub fn load_datum<'file>(&mut self, layout: &FileLayout) -> DatumResult<DatumPage<'d>>
    where
        'd: 'file,
    {
        let (header, required_layout) = Self::calculate_page_size(layout)?;
        let base = self
            .allocator
            .raw_alloc(required_layout)
            .ok_or(DatumspaceError::AllocationFailure)?;

        let page = unsafe {
            let builder = || -> Option<_> {
                let page_builder = PageBuilder::new(base, header)
                    .write_code_blob(&layout.code_directory.bytecode)?
                    .write_data_blob(&layout.data_directory.data)?
                    .write_constants(layout.data_directory.entries.iter().map(|x| {
                        ConstantTableEntry::Unresolved(DataEntry {
                            loc: (header.data_blob.0 + Offset(x.index), x.length),
                            tag: x.type_tag,
                        })
                    }))?
                    .write_functions(layout.code_directory.functions.iter().map(|x| {
                        Runnable::Function(Function {
                            maxstack: x.maxstack,
                            maxlocals: x.maxlocals,
                            bytecode: (header.bytecode_blob.0 + Offset(x.index), x.length),
                            flags: FunctionFlags::from_bits_retain(x.flags.bits()),
                            param_count: x.param_count,
                        })
                    }))?
                    .write_symbols(layout.symbol_table.symbols.iter().map(|x| Symbol {
                        kind: match x.kind
                        {
                            ParsedSymbolKind::Function { body } => SymbolKind::Function { index: body },
                            ParsedSymbolKind::Type {} => todo!(),
                        },
                        id: x.id,
                    }))?;

                let iter = layout.link_table.entries.iter().map(|x| {
                    Ok::<_, ()>(Link {
                        id: x.module_id,
                        path: {
                            let DataHeader {
                                length,
                                index,
                                type_tag: _,
                            } = layout
                                .data_directory
                                .entries
                                .get(<usize>::try_from(x.module_path).map_err(|_| ())?)
                                .ok_or(())?;

                            // TODO: Ensure type tag is a string to prevent malformations
                            InlinedString::new((header.data_blob.0 + Offset(*index), *length))
                        },
                    })
                });

                process_results(iter, |i| page_builder.write_links(i)).ok()?
            };

            // Technically this will result in the allocated page being leaked, were the error case to be reached
            // but realistically this situation is just going to end up with the entire runtime shutting down anyway
            builder().ok_or(DatumspaceError::InvalidStructure)?.resolve()
        };

        self.insert_mapping(*page.id, base)?;
        Ok(page)
    }

    pub fn get_page(&self, id: &SymbolId) -> DatumResult<DatumPage<'d>>
    {
        self.mapping
            .get(id)
            .map(|x| unsafe { DatumPage::from_base_ptr(*x) })
            .ok_or(DatumspaceError::PageNotLoaded)
    }

    pub fn get_function(&self, page_id: &SymbolId, index: usize) -> Result<&'d Function, DatumspaceError>
    {
        let page = self.get_page(page_id)?;
        let runnable = page.functions.get(index).ok_or(DatumspaceError::ResourceDoesntExist)?;

        match runnable
        {
            Runnable::Function(f) => Ok(f),
        }
    }

    pub fn get_functions(&self, page_id: &SymbolId) -> DatumResult<impl Iterator<Item = &'d Function> + 'd>
    {
        let page = self.get_page(page_id)?;
        Ok(page.functions.iter().filter_map(|x| match x
        {
            Runnable::Function(f) => Some(f),
            _ => None,
        }))
    }

    pub fn get_constant(&mut self, page_id: &SymbolId, index: usize) -> DatumResult<&'d Constant>
    {
        let page = self.get_page(page_id)?;

        let entry = page.constants.get(index).ok_or(DatumspaceError::ResourceDoesntExist)?;
        match entry
        {
            ConstantTableEntry::Resolved(constant) => Ok(&constant),
            ConstantTableEntry::Unresolved(entry) =>
            unsafe {
                self.write_constant(
                    page_id,
                    index,
                    Constant::from_entry(*self.mapping.get(page_id).ok_or(DatumspaceError::PageNotLoaded)?, entry)
                        .ok_or(DatumspaceError::InvalidConstantType)?,
                )
            },
        }
    }

    unsafe fn write_constant(
        &mut self,
        page_id: &SymbolId,
        index: usize,
        constant: Constant,
    ) -> DatumResult<&'d Constant>
    {
        let base = self.mapping.get(page_id).ok_or(DatumspaceError::PageNotLoaded)?;
        let header: NonNull<DatumPageHeader> = base.cast();
        let constants_loc = unsafe { header.read().constants };

        assert!(index < <usize>::try_from(constants_loc.1).unwrap() / size_of::<Constant>());

        Ok(unsafe {
            let ptr = constants_loc.0.as_ptr(*base).add(index);
            ptr.write(constant);

            ptr.as_ref()
        })
    }

    pub fn resolve_location(&self, page_id: &SymbolId, loc: BlockLocation) -> Result<&'d [u8], DatumspaceError>
    {
        self.mapping
            .get(page_id)
            .map(|x| unsafe { NonNull::slice_from_raw_parts(loc.0.as_ptr(*x), loc.1 as usize).as_ref() })
            .ok_or(DatumspaceError::ResourceDoesntExist)
    }

    pub fn resolve_location_mut(
        &mut self,
        page_id: &SymbolId,
        loc: BlockLocation,
    ) -> Result<&'d mut [u8], DatumspaceError>
    {
        self.mapping
            .get(page_id)
            .map(|x| unsafe { NonNull::slice_from_raw_parts(loc.0.as_ptr(*x), loc.1 as usize).as_mut() })
            .ok_or(DatumspaceError::ResourceDoesntExist)
    }

    pub fn resolve_string<'a>(&self, page_id: &SymbolId, string: &'a InlinedString)
    -> Result<&'a str, DatumspaceError>
    {
        self.mapping
            .get(page_id)
            .and_then(|x| unsafe { string.get(*x) })
            .ok_or(DatumspaceError::ResourceDoesntExist)
    }

    fn calculate_page_size<'file>(layout: &FileLayout) -> Result<(DatumPageHeader, Layout), DatumspaceError>
    {
        // link table
        // Each link gets slightly flattened, removing now unrequired metadata
        let link_table_size = layout.link_table.entries.len() * size_of::<Link>();

        // symbol table
        let symbol_table_size = layout.symbol_table.symbols.len() * size_of::<Symbol>();

        // function table
        let function_table_size = layout.code_directory.function_count() * size_of::<Runnable>();

        // constant table
        let constant_table_size = layout.data_directory.entries.len() * size_of::<Constant>();

        // Code size
        let code_size = layout.code_directory.bytecode_size();

        // Data size
        let data_size = layout.data_directory.data_byte_size();

        let (link_table_loc, symbol_table_loc, function_table_loc, constant_table_loc, code_loc, data_loc) = [
            link_table_size,
            symbol_table_size,
            function_table_size,
            constant_table_size,
            code_size,
            data_size,
        ]
        .iter()
        .scan(size_of::<DatumPageHeader>(), |cursor, size| {
            let start = *cursor;
            *cursor += size;

            Some((Offset(start.try_into().ok()?), (*size).try_into().ok()?))
        })
        .collect_tuple()
        .ok_or(DatumspaceError::InvalidStructure)?;

        let header = DatumPageHeader {
            id: layout.header.module_id,
            link_table: link_table_loc,
            symbol_table: symbol_table_loc,
            functions: function_table_loc,
            constants: constant_table_loc,
            bytecode_blob: code_loc,
            data_blob: data_loc,
        };

        let size = data_loc
            .0
            .0
            .checked_add(data_loc.1)
            .ok_or(DatumspaceError::InvalidStructure)
            .and_then(|x| <usize>::try_from(x).map_err(|_| DatumspaceError::InvalidStructure))?;

        let layout = Layout::from_size_align(size, align_of::<DatumPageHeader>())
            .map_err(|_| DatumspaceError::AllocationFailure)?;

        Ok((header, layout))
    }

    fn insert_mapping(&mut self, key: SymbolId, page: NonNull<u8>) -> Result<(), DatumspaceError>
    {
        // Use the entry function to prevent overwritting existing data in case of duplication
        match self.mapping.entry(key)
        {
            Entry::Vacant(entry) =>
            {
                entry.insert(page);
                Ok(())
            }
            Entry::Occupied(_) => Err(DatumspaceError::Duplication),
        }
    }
}

// TODO: Write tests for this
