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

use itertools::{Itertools as _, process_results};

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
            ConstantTableEntry::Resolved(constant) => Ok(constant),
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

#[cfg(test)]
mod tests
{
    use super::*;
    use crate::loader::parser::layout::{
        CodeDirectory, DataDirectory, DataHeader, FileFlags, FileHeader, Function, FunctionFlags, LinkTable,
        SymbolEntry, SymbolKind, SymbolTable, TypeTag,
    };

    /// Helper to create a dummy FileLayout for testing based on the new structure.
    fn mock_layout(id: SymbolId, code: Vec<u8>, data: Vec<u8>) -> FileLayout
    {
        FileLayout {
            header: FileHeader {
                module_id: id,
                file_version: 1,
                min_runtime_version: 1,
                flags: FileFlags::empty(),
            },
            link_table: LinkTable { entries: vec![] },
            symbol_table: SymbolTable {
                symbols: vec![SymbolEntry {
                    id,
                    kind: SymbolKind::Function { body: 0 },
                }],
            },
            code_directory: CodeDirectory {
                functions: vec![Function {
                    symbol_id: id,
                    index: 0,
                    length: code.len() as u32,
                    maxlocals: 5,
                    maxstack: 10,
                    param_count: 2,
                    flags: FunctionFlags::empty(),
                }],
                bytecode: code,
            },
            data_directory: DataDirectory {
                entries: vec![DataHeader {
                    index: 0,
                    length: data.len() as u32,
                    type_tag: TypeTag::Integer32,
                }],
                data,
            },
        }
    }

    #[test]
    fn test_datumspace_initialization()
    {
        let ds = Datumspace::with_capacity(1024);
        assert!(ds.is_ok());
    }

    #[test]
    fn test_load_and_retrieve_page()
    {
        let mut ds = Datumspace::with_capacity(4096).unwrap();
        let id = SymbolId([0; 16]);
        let layout = mock_layout(id, vec![0x01, 0x02], vec![0xAA, 0xBB, 0xCC, 0xDD]);

        let load_result = ds.load_datum(&layout);
        assert!(load_result.is_ok());

        let page = ds.get_page(&id).expect("Page should be loaded");
        assert_eq!(*page.id, id);

        // Verify code and data blobs are correctly mapped via the resolved page
        assert_eq!(page.bytecode_blob, &[0x01, 0x02]);
        assert_eq!(page.data_blob, &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn test_duplicate_load_fails()
    {
        let mut ds = Datumspace::with_capacity(4096).unwrap();
        let id = SymbolId([0; 16]);
        let layout = mock_layout(id, vec![0], vec![0, 0, 0, 0]);

        ds.load_datum(&layout).expect("First load should succeed");
        let result = ds.load_datum(&layout);

        assert!(matches!(result, Err(DatumspaceError::Duplication)));
    }

    #[test]
    fn test_function_retrieval()
    {
        let mut ds = Datumspace::with_capacity(4096).unwrap();
        let id = SymbolId([0; 16]);
        let layout = mock_layout(id, vec![0xDE, 0xAD], vec![0, 0, 0, 0]);

        ds.load_datum(&layout).unwrap();
        let func = ds.get_function(&id, 0).expect("Should find function at index 0");

        assert_eq!(func.maxstack, 10);
        assert_eq!(func.param_count, 2);

        // Verify the function's bytecode location resolves to the correct bytes
        let code = ds.resolve_location(&id, func.bytecode).unwrap();
        assert_eq!(code, &[0xDE, 0xAD]);
    }

    #[test]
    fn test_lazy_constant_resolution()
    {
        let mut ds = Datumspace::with_capacity(4096).unwrap();
        let id = SymbolId([0; 16]);
        // 42 in Little Endian for Unsigned32/Integer32
        let data = vec![42, 0, 0, 0];
        let layout = mock_layout(id, vec![0], data);

        ds.load_datum(&layout).unwrap();

        // 1. Check initial state via the page (should be Unresolved)
        {
            let page = ds.get_page(&id).unwrap();
            match page.constants[0]
            {
                ConstantTableEntry::Unresolved(_) =>
                {}
                _ => panic!("Constant should start as Unresolved"),
            }
        }

        // 2. Resolve the constant via the Datumspace
        let constant = ds.get_constant(&id, 0).expect("Failed to resolve constant");
        if let Constant::Unsigned32(val) = constant
        {
            assert_eq!(*val, 42);
        }
        else
        {
            panic!("Expected Unsigned32 constant, got {:?}", constant);
        }

        // 3. Verify it is now mutated to Resolved in memory
        let page = ds.get_page(&id).unwrap();
        match page.constants[0]
        {
            ConstantTableEntry::Resolved(c) =>
            {
                if let Constant::Unsigned32(v) = c
                {
                    assert_eq!(v, 42);
                }
            }
            _ => panic!("Constant should be Resolved in-place"),
        }
    }

    #[test]
    fn test_resolve_string()
    {
        let mut ds = Datumspace::with_capacity(4096).unwrap();
        let id = SymbolId([0; 16]);
        let string_text = "Azimuth";
        let string_data = string_text.as_bytes().to_vec();

        let mut layout = mock_layout(id, vec![0], string_data);
        layout.data_directory.entries[0].type_tag = TypeTag::String;

        ds.load_datum(&layout).unwrap();

        let constant = ds.get_constant(&id, 0).unwrap();
        if let Constant::String(inlined) = constant
        {
            let resolved = ds.resolve_string(&id, inlined).unwrap();
            assert_eq!(resolved, string_text);
        }
        else
        {
            panic!("Expected String constant");
        }
    }

    #[test]
    fn test_allocation_failure_on_small_capacity()
    {
        // Try to allocate a space that is clearly too small for the headers and tables
        let mut ds = Datumspace::with_capacity(64).unwrap();
        let layout = mock_layout(SymbolId([0; 16]), vec![0; 512], vec![0; 512]);

        let result = ds.load_datum(&layout);
        assert!(matches!(result, Err(DatumspaceError::AllocationFailure)));
    }
}
