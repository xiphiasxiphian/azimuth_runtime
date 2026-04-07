pub mod tables;
pub mod datum;
pub mod runnable;
pub mod types;

use std::{
    alloc::Layout, collections::{HashMap, hash_map::Entry}, ops::AddAssign, ptr::NonNull
};

use itertools::Itertools;

use crate::{
    loader::{SymbolId, parser::{
        function::{self, Directive, FunctionInfo}, layout::{FileLayout, Link as ParsedLink}, table::TableEntry
    }},
    memory::{
        allocators::{AllocatorError, general::GeneralAllocator},
        datumspace::{
            datum::{BlockLocation, DatumPage, DatumPageHeader, Offset, PageBuilder, align_up},
            runnable::Runnable, tables::{constant_table::{Constant, ConstantTableEntry, DataEntry}, link_table::{self, Link}, symbol_table::Symbol},
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
    Page(&'a DatumPageHeader),
    Function(&'a Runnable<'a>),
    Type(),
}

pub struct Datumspace<'a>
{
    allocator: DatumAllocator,
    mapping: HashMap<&'a str, DatumEntry<'a>>,
}

impl<'d> Datumspace<'d>
{
    pub fn with_capacity(min_capacity: usize) -> Result<Self, AllocatorError>
    {
        Ok(Self {
            allocator: DatumAllocator::with_capacity(min_capacity)?,
            mapping: HashMap::new(),
        })
    }

    pub fn load_datum<'file>(
        &mut self,
        layout: &FileLayout,
    ) -> Result<DatumPage<'d>, DatumspaceError>
    where
        'd: 'file,
    {
        let (header, required_layout) = Self::calculate_page_size(layout)?;
        let base = self.allocator.raw_alloc(required_layout).ok_or(DatumspaceError::AllocationFailure)?;

        let page = unsafe {
            let builder = || -> Option<_> {
                PageBuilder::new(base, header)
                    .write_code_blob(&layout.code_directory.bytecode)?
                    .write_data_blob(&layout.data_directory.data)?
                    .write_constants(
                        layout.data_directory.entries.iter().map(|x| {
                            ConstantTableEntry::Unresolved(
                                DataEntry {
                                    loc: (header.data_blob.0 + Offset(x.index), x.length)
                                }
                            )
                        })
                    )
            };

            builder().ok_or(DatumspaceError::InvalidStructure)?.resolve()
        };

    }

    pub fn get_page(&self, id: &str) -> Result<DatumPage<'d>, DatumspaceError>
    {
        match self.mapping.get(id)
        {
            Some(&DatumEntry::Page(header)) => Ok(unsafe { header.get_page() }),
            Some(_) => Err(DatumspaceError::UnexpectedDatumtype),
            None => Err(DatumspaceError::ResourceDoesntExist),
        }
    }

    pub fn get_constants(&self, id: &str) -> Result<&'d [Constant<'d>], DatumspaceError>
    {
        // The main difference here is that the id refers to the datumpage rather than a specific entry in it, as
        // every page only has one constant table.

        self.get_page(id).map(|x| x.constants)
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
        layout: &FileLayout,
    ) -> Result<(DatumPageHeader, Layout), DatumspaceError>
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

        let (
            link_table_loc,
            symbol_table_loc,
            function_table_loc,
            constant_table_loc,
            code_loc,
            data_loc,
        ) = [
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

        let size = data_loc.0.0.checked_add(data_loc.1)
            .ok_or(DatumspaceError::InvalidStructure)
            .and_then(|x| <usize>::try_from(x).map_err(|_| DatumspaceError::InvalidStructure))?;

        let layout = Layout::from_size_align(size, align_of::<DatumPageHeader>()).map_err(|_| DatumspaceError::AllocationFailure)?;

        Ok((header, layout))
    }

    fn insert_mapping(&mut self, key: &'d str, datumentry: DatumEntry<'d>) -> Result<&mut DatumEntry<'d>, DatumspaceError>
    {
        // Use the entry function to prevent overwritting existing data in case of duplication
        match self.mapping.entry(key)
        {
            Entry::Vacant(entry) =>
            {
                Ok(entry.insert(datumentry))
            }
            Entry::Occupied(_) => Err(DatumspaceError::Duplication),
        }
    }
}

#[cfg(test)]
mod datumspace_tests
{
    use super::*;
    use crate::loader::parser::{
        function::{Directive, FunctionInfo},
        table::TableEntry,
    };

    /// A minimal allocator capacity sufficient for all happy-path tests.
    const TEST_CAPACITY: usize = 4096;

    /// Builds a table with one string entry (used as the function name) plus
    /// a handful of primitives, covering every Constant variant.
    fn make_table<'a>(name: &'a str) -> Vec<TableEntry<'a>>
    {
        vec![
            TableEntry::String(name), // index 0 — used as name_index
            TableEntry::Integer(42),
            TableEntry::Long(9999),
            TableEntry::Float(1.5),
            TableEntry::Double(2.71),
        ]
    }

    /// Minimal valid directives: MaxStack + MaxLocals are the two required ones.
    /// `from_parsed_data` strips them out, so any extras go into the directive blob.
    fn make_directives() -> Vec<Directive>
    {
        vec![Directive::MaxStack(8), Directive::MaxLocals(4)]
    }

    fn make_function<'a>(name_index: usize, code: &'a [u8]) -> FunctionInfo<'a>
    {
        FunctionInfo {
            name_index,
            directives: make_directives(),
            code,
        }
    }

    fn make_datumspace<'a>() -> Datumspace<'a>
    {
        Datumspace::with_capacity(TEST_CAPACITY).expect("allocator init failed")
    }

    // #[test]
    // fn load_datum_happy_path()
    // {
    //     let mut ds = make_datumspace();
    //     let table = make_table("my_func");
    //     let code = vec![0x01, 0x02, 0x03];
    //     let functions = vec![make_function(0, &code)];

    //     let page = ds
    //         .load_datum("my_datum", &table, &functions)
    //         .expect("load_datum should succeed");

    //     assert_eq!(page.id, "my_datum");
    //     assert_eq!(page.constants.len(), table.len());
    //     assert_eq!(page.functions.len(), functions.len());
    // }

    // #[test]
    // fn get_constant_returns_correct_values()
    // {
    //     let mut ds = make_datumspace();
    //     let table = make_table("fn_name");
    //     let functions = vec![make_function(0, &[0xAB])];

    //     ds.load_datum("datum_a", &table, &functions).unwrap();

    //     // index 0 — the interned string
    //     assert!(matches!(
    //         ds.get_constants("datum_a").unwrap()[0],
    //         Constant::String(s) if s == "fn_name"
    //     ));

    //     // index 1 — integer primitive
    //     assert!(matches!(
    //         ds.get_constants("datum_a").unwrap()[1],
    //         Constant::Unsigned32(42)
    //     ));

    //     // index 2 — long primitive
    //     assert!(matches!(
    //         ds.get_constants("datum_a").unwrap()[2],
    //         Constant::Unsigned64(9999)
    //     ));
    // }

    // #[test]
    // fn get_runnable_returns_correct_function()
    // {
    //     let mut ds = make_datumspace();
    //     let table = make_table("entry");
    //     let code = vec![0xDE, 0xAD, 0xBE, 0xEF];
    //     let functions = vec![make_function(0, &code)];

    //     ds.load_datum("datum_b", &table, &functions).unwrap();

    //     let runnable = ds.get_runnable("entry").expect("runnable should exist");
    //     assert_eq!(runnable.name, "entry");
    //     assert_eq!(runnable.bytecode, &[0xDE, 0xAD, 0xBE, 0xEF]);
    //     assert_eq!(runnable.maxstack, 8);
    //     assert_eq!(runnable.maxlocals, 4);
    // }

    // #[test]
    // fn duplicate_datum_id_is_rejected()
    // {
    //     let mut ds = make_datumspace();
    //     let table = make_table("func");
    //     let functions = vec![make_function(0, &[0x00])];

    //     ds.load_datum("same_id", &table, &functions).unwrap();

    //     let table2 = make_table("func2");
    //     let functions2 = vec![make_function(0, &[0x01])];
    //     let result = ds.load_datum("same_id", &table2, &functions2);

    //     assert!(matches!(result, Err(DatumspaceError::Duplication)));
    // }

    // #[test]
    // fn duplicate_function_name_across_datums_is_rejected()
    // {
    //     let mut ds = make_datumspace();

    //     // First datum registers "shared_fn"
    //     let table1 = make_table("shared_fn");
    //     let fns1 = vec![make_function(0, &[0x01])];
    //     ds.load_datum("datum_one", &table1, &fns1).unwrap();

    //     // Second datum also tries to register "shared_fn"
    //     let table2 = make_table("shared_fn");
    //     let fns2 = vec![make_function(0, &[0x02])];
    //     let result = ds.load_datum("datum_two", &table2, &fns2);

    //     assert!(matches!(result, Err(DatumspaceError::Duplication)));
    // }

    // #[test]
    // fn non_string_name_index_is_rejected()
    // {
    //     let mut ds = make_datumspace();
    //     // Table where index 0 is an integer, not a string
    //     let table = vec![TableEntry::Integer(99), TableEntry::String("real_name")];
    //     // name_index 0 points at the Integer — should fail
    //     let functions = vec![make_function(0, &[0x00])];

    //     let result = ds.load_datum("datum_bad_type", &table, &functions);
    //     assert!(matches!(result, Err(DatumspaceError::UnexpectedDatumtype)));
    // }

    // #[test]
    // fn out_of_bounds_name_index_is_rejected()
    // {
    //     let mut ds = make_datumspace();
    //     let table = make_table("some_fn"); // 5 entries, indices 0..=4
    //     // name_index 99 is well out of range
    //     let functions = vec![make_function(99, &[0x00])];

    //     let result = ds.load_datum("datum_oob", &table, &functions);
    //     assert!(matches!(result, Err(DatumspaceError::ResourceDoesntExist)));
    // }
}
