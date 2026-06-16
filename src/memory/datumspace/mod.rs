pub mod datum;
pub mod layout_engine;
pub mod runnable;
pub mod tables;

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
        parser::layout::{
            DataHeader, FileLayout,
            SymbolKind::{self as ParsedSymbolKind, Type},
            UserDefinedType,
        },
    },
    memory::{
        allocators::{AllocatorError, general::GeneralAllocator},
        datumspace::{
            datum::{BlockLocation, DatumPage, DatumPageHeader, InlinedString, Offset, PageBuilder},
            layout_engine::{LayoutEngine, TypeLayout},
            runnable::{Function, FunctionFlags, Runnable},
            tables::{
                constant_table::{Constant, ConstantTableEntry, ConstantTableEntryData, DataEntry},
                link_table::{self, Link},
                symbol_table::{Symbol, SymbolKind},
                types::{RuntimeEnumVariant, RuntimeType, RuntimeTypeKind},
            },
        },
        heap::heap::ObjectHeader,
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
        let (mut runtime_types, runtime_variants, runtime_gc_offsets) = self.compute_runtime_layouts(layout)?;

        let (header, required_layout) = Self::calculate_page_size(
            layout,
            runtime_types.len(),
            runtime_variants.len(),
            runtime_gc_offsets.len(),
        )?;

        let base = self
            .allocator
            .raw_alloc(required_layout)
            .ok_or(DatumspaceError::AllocationFailure)?;

        let page = unsafe {
            let builder = || -> Option<_> {
                let page_builder = PageBuilder::new(base, header)
                    .write_code_blob(&layout.code_directory.bytecode)?
                    .write_data_blob(&layout.data_directory.data)?
                    .write_types(runtime_types.iter_mut().map(|x| {
                        x.back_pointer = base.cast();
                        *x
                    }))? // fix all the back pointers
                    .write_enum_variants(runtime_variants.into_iter())?
                    .write_gc_offsets(runtime_gc_offsets.into_iter())?
                    .write_constants(layout.data_directory.entries.iter().map(|x| {
                        ConstantTableEntry::new(ConstantTableEntryData::Unresolved(DataEntry {
                            loc: (header.data_blob.0 + Offset(x.index), x.length),
                            tag: x.signature,
                        }))
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
                            ParsedSymbolKind::Type { type_index } => SymbolKind::Type { index: type_index },
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
                                signature: _,
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
        match entry.as_data()
        {
            ConstantTableEntryData::Resolved(constant) => Ok(constant),
            ConstantTableEntryData::Unresolved(entry) =>
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

    /// Queries a loaded DatumPage for the exact pre-calculated physical layout
    /// of an exported type, allowing an external module's LayoutEngine to embed it.
    pub fn get_external_layout(&self, page_id: &SymbolId, type_index: u32) -> DatumResult<TypeLayout>
    {
        let page = self.get_page(page_id)?;

        let runtime_type = page
            .types
            .get(type_index as usize)
            .ok_or(DatumspaceError::ResourceDoesntExist)?;

        match runtime_type.kind
        {
            RuntimeTypeKind::Struct {
                instance_size,
                alignment,
                gc_offsets_count,
                ..
            } => Ok(TypeLayout {
                size: instance_size as u32,
                align: alignment,
                has_gc_roots: gc_offsets_count > 0,
            }),
            RuntimeTypeKind::Enum {
                variants_index,
                variants_count,
                instance_size,
                alignment,
            } =>
            {
                // Enums cache their overall size/alignment, but we still check
                // if any individual variant introduces a GC root pointer.
                let start = variants_index as usize;
                let end = start + variants_count as usize;
                let variants = page
                    .enum_variants
                    .get(start..end)
                    .ok_or(DatumspaceError::InvalidStructure)?;

                let has_gc_roots = variants.iter().any(|v| v.gc_offsets_count > 0);

                Ok(TypeLayout {
                    size: instance_size as u32,
                    align: alignment,
                    has_gc_roots,
                })
            }
            RuntimeTypeKind::Imported { module_id, type_index } =>
            {
                todo!() // go play fetch another time
            }
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
        let header: &DatumPageHeader = unsafe { base.cast().as_ref() };
        let constants_loc = header.constants;

        assert!(
            index < <usize>::try_from(constants_loc.1).unwrap() / size_of::<ConstantTableEntry>(),
            "Constant index out of bounds"
        );

        // yeah should probably UnsafeCell a bunch of this but oh well, ill fix that later
        Ok(unsafe {
            let ptr = constants_loc.0.as_ptr(*base).add(index);
            ptr.write(ConstantTableEntry::new(ConstantTableEntryData::Resolved(constant)));

            match ptr.as_ref().as_data() {
                ConstantTableEntryData::Resolved(c) => c,
                _ => unreachable!(),
            }
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

    fn calculate_page_size(
        layout: &FileLayout,
        num_types: usize,
        num_variants: usize,
        num_gc_offsets: usize,
    ) -> DatumResult<(DatumPageHeader, Layout)>
    {
        let mut current_offset = size_of::<DatumPageHeader>();
        let mut max_align = align_of::<DatumPageHeader>();

        // Helper macro to align the current offset up to T's alignment requirement
        macro_rules! align_section {
            ($align_ty:ty, $count:expr) => {{
                let align = std::mem::align_of::<$align_ty>();
                max_align = max_align.max(align);

                // Round up to the nearest multiple of alignment
                current_offset = (current_offset + align - 1) & !(align - 1);

                let start = current_offset;
                let bytes = $count * std::mem::size_of::<$align_ty>();
                current_offset += bytes;

                (Offset(start as u32), bytes as u32)
            }};
        }

        // Compute padded locations for every section
        let link_table     = align_section!(Link, layout.link_table.entries.len());
        let symbol_table   = align_section!(Symbol, layout.symbol_table.symbols.len());
        let functions      = align_section!(Runnable, layout.code_directory.functions.len());
        let constants      = align_section!(ConstantTableEntry, layout.data_directory.entries.len());
        let types          = align_section!(RuntimeType, num_types);
        let enum_variants  = align_section!(RuntimeEnumVariant, num_variants);
        let gc_offsets     = align_section!(usize, num_gc_offsets);
        let bytecode_blob  = align_section!(u8, layout.code_directory.bytecode.len());
        let data_blob      = align_section!(u8, layout.data_directory.data.len());

        let header = DatumPageHeader {
            id: layout.header.module_id,
            link_table,
            symbol_table,
            constants,
            functions,
            types,
            enum_variants,
            gc_offsets,
            bytecode_blob,
            data_blob,
        };

        current_offset = (current_offset + max_align - 1) & !(max_align - 1);

        // Create the allocation layout with the maximum required alignment
        let required_layout = Layout::from_size_align(current_offset, max_align)
            .map_err(|_| DatumspaceError::InvalidStructure)?;

        Ok((header, required_layout))
    }

    /// Computes the runtime sizes and flattens the GC offsets for all types.
    fn compute_runtime_layouts(
        &self,
        layout: &FileLayout,
    ) -> DatumResult<(Vec<RuntimeType>, Vec<RuntimeEnumVariant>, Vec<usize>)>
    {
        let type_count = layout.type_directory.types.len();
        let mut runtime_types = Vec::with_capacity(type_count);
        let mut runtime_variants = Vec::new();
        let mut gc_offsets = Vec::new();

        // initializes zero-allocation buffer for layout resolution
        let mut engine = LayoutEngine::new(type_count);

        for (index, udt) in layout.type_directory.types.iter().enumerate()
        {
            match udt
            {
                UserDefinedType::Struct(s) =>
                {
                    let gc_offsets_index = gc_offsets.len() as u32;

                    // delegates layout resolution and internal caching to the engine
                    let heap_layout =
                        engine.resolve_struct(index, &s.fields, &mut gc_offsets, size_of::<ObjectHeader>() as u32)?;

                    runtime_types.push(RuntimeType {
                        back_pointer: NonNull::dangling(), // This will be later inited
                        symbol_id: s.symbol_id,
                        kind: RuntimeTypeKind::Struct {
                            instance_size: heap_layout.size as usize,
                            alignment: heap_layout.align,
                            gc_offsets_index,
                            gc_offsets_count: (gc_offsets.len() as u32) - gc_offsets_index,
                        },
                    });
                }
                UserDefinedType::Enum(e) =>
                {
                    let variants_index = runtime_variants.len() as u32;
                    let mut variant_layouts = Vec::with_capacity(e.variants.len());

                    for variant in &e.variants
                    {
                        let gc_offsets_index = gc_offsets.len() as u32;

                        // resolves the physical structure of the variant fields
                        let layout = engine.resolve_variant(
                            &variant.fields,
                            &mut gc_offsets,
                            size_of::<ObjectHeader>() as u32,
                        )?;

                        runtime_variants.push(RuntimeEnumVariant {
                            tag: variant.tag,
                            gc_offsets_index,
                            gc_offsets_count: (gc_offsets.len() as u32) - gc_offsets_index,
                        });

                        variant_layouts.push(layout);
                    }

                    // finishes the enum by determining maximum bounds and updating the cache
                    let heap_enum_layout = engine.finalize_enum(index, &variant_layouts);

                    runtime_types.push(RuntimeType {
                        back_pointer: NonNull::dangling(),
                        symbol_id: e.symbol_id,
                        kind: RuntimeTypeKind::Enum {
                            variants_index,
                            variants_count: e.variants.len() as u32,
                            instance_size: heap_enum_layout.size as usize,
                            alignment: heap_enum_layout.align,
                        },
                    });
                }
                UserDefinedType::Imported {
                    local_id,
                    link_index,
                    target_id,
                } =>
                {
                    let link = layout
                        .link_table
                        .entries
                        .get(*link_index as usize)
                        .ok_or(DatumspaceError::ResourceDoesntExist)?;

                    let type_index = match layout
                        .symbol_table
                        .symbols
                        .iter()
                        .find(|x| &x.id == target_id)
                        .ok_or(DatumspaceError::ResourceDoesntExist)?
                        .kind
                    {
                        Type { type_index } => Ok(type_index),
                        _ => Err(DatumspaceError::InvalidStructure),
                    }?;

                    let external_layout = self.get_external_layout(&link.module_id, type_index)?;
                    engine.cache_result(external_layout, index);

                    // Push a proxy/reference to maintain 1:1 index alignment
                    // in your runtime_types array.
                    runtime_types.push(RuntimeType {
                        back_pointer: NonNull::dangling(),
                        symbol_id: *local_id, // the ID it uses in THIS module
                        kind: RuntimeTypeKind::Imported {
                            // Store enough metadata to forward allocations/method calls
                            // to the external module when encountered at runtime.
                            module_id: link.module_id.clone(),
                            type_index: type_index,
                        },
                    });
                }
            }
        }

        Ok((runtime_types, runtime_variants, gc_offsets))
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
        CodeDirectory, ConstantSignature, DataDirectory, DataHeader, FileFlags, FileHeader, Function, FunctionFlags,
        LinkTable, ScalarTag, SymbolEntry, SymbolKind, SymbolTable, TypeDirectory,
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
            type_directory: TypeDirectory {
                types: vec![], // TODO: Type tests in datumspace
            },
            data_directory: DataDirectory {
                entries: vec![DataHeader {
                    index: 0,
                    length: data.len() as u32,
                    signature: ConstantSignature::Scalar(ScalarTag::Integer32),
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
            match page.constants[0].as_data()
            {
                ConstantTableEntryData::Unresolved(_) =>
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
        match page.constants[0].as_data()
        {
            ConstantTableEntryData::Resolved(c) =>
            {
                if let &Constant::Unsigned32(v) = c
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
        layout.data_directory.entries[0].signature = ConstantSignature::String;

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
