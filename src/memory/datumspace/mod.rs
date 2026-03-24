pub mod constant_table;
mod datum;
pub mod runnable;
pub mod types;

use std::{
    alloc::Layout,
    collections::{HashMap, hash_map::Entry},
    ptr::NonNull,
};

use crate::{
    loader::parser::{
        function::{Directive, FunctionInfo},
        table::{Table, TableEntry},
    },
    memory::{
        allocators::{AllocatorError, general::GeneralAllocator},
        datumspace::{
            constant_table::Constant,
            datum::{DatumPage, DatumPageHeader, align_up},
            runnable::Runnable,
            types::TypeInfo,
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
        id: &'file str,
        table: &'file [TableEntry<'file>],
        functions: &'file [FunctionInfo<'file>],
    ) -> Result<DatumPage<'d>, DatumspaceError>
    where
        'd: 'file,
    {
        let page_layout = Self::calculate_page_size(id, table, functions)?;
        let base: NonNull<u8> = self
            .allocator
            .raw_alloc(page_layout)
            .ok_or(DatumspaceError::AllocationFailure)?;

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
        let const_offset = align_up(size_of::<DatumPageHeader>() + id.len(), align_of::<Constant>());

        let fn_offset = align_up(
            const_offset + table.len() * size_of::<Constant>(),
            align_of::<Runnable>(),
        );

        // Blob cursor begins after the functions array
        let mut blob_cursor = fn_offset + functions.len() * size_of::<Runnable>();

        // Write id to start
        let id: &'d str = unsafe {
            let id_dest = base.byte_add(size_of::<DatumPageHeader>());
            std::ptr::copy_nonoverlapping(id.as_ptr(), id_dest.as_ptr(), id.len());

            str::from_utf8_unchecked(std::slice::from_raw_parts(id_dest.as_ref(), id.len()))
        };

        // Write constants
        let const_base = unsafe { base.byte_add(const_offset).cast() };

        for (i, entry) in table.iter().enumerate()
        {
            let constant: Constant<'d> = match entry
            {
                TableEntry::Integer(v) => Constant::Unsigned32(*v),
                TableEntry::Long(v) => Constant::Unsigned64(*v),
                TableEntry::Float(v) => Constant::Float32(*v),
                TableEntry::Double(v) => Constant::Float64(*v),
                TableEntry::String(s) =>
                {
                    // Write blob at cursor, produce a 'd slice into the page
                    let blob_ptr = unsafe { base.byte_add(blob_cursor) };
                    unsafe {
                        blob_ptr.copy_from_nonoverlapping(NonNull::new_unchecked(s.as_ptr() as *mut _), s.len());
                    }
                    let permanent: &'d str =
                        unsafe { str::from_utf8_unchecked(NonNull::slice_from_raw_parts(blob_ptr, s.len()).as_ref()) };
                    blob_cursor += s.len();
                    Constant::String(permanent)
                }
            };
            unsafe { const_base.add(i).write(constant) };
        }

        // Write functions
        let fn_base = unsafe { base.byte_add(fn_offset).cast::<Runnable<'d>>() };
        let constants: &'d [Constant<'d>] = unsafe { std::slice::from_raw_parts(const_base.as_ptr(), table.len()) };

        for (i, info) in functions.iter().enumerate()
        {
            // Resolve name from the constants we just wrote
            let name = match constants.get(info.name_index)
            {
                Some(Constant::String(s)) => *s,
                Some(_) => return Err(DatumspaceError::UnexpectedDatumtype),
                None => return Err(DatumspaceError::ResourceDoesntExist),
            };

            // Write directives blob
            blob_cursor = align_up(blob_cursor, align_of::<Directive>());
            let dir_ptr = unsafe { base.byte_add(blob_cursor).cast::<Directive>() };
            let dir_len = info.directives.len();

            // TODO: Technically some directives are removed as they are the required ones
            blob_cursor += dir_len * size_of::<Directive>();

            // Write bytecode blob
            let code_ptr = unsafe { base.byte_add(blob_cursor) };
            let code_len = info.code.len();

            blob_cursor += code_len;

            // Build the Runnable directly into the page — no allocator call needed
            // since directives and bytecode now live in the page block itself.
            let runnable = unsafe {
                Runnable::from_parsed_data(fn_base.add(i), dir_ptr, code_ptr, name, &info.directives, info.code)?
            };

            // Register name -> function pointer in the flat lookup map
            self.insert_mapping(name, DatumEntry::Function(runnable))?;
        }

        let page = unsafe { DatumPage::from_base_ptr(base.as_ptr() as *const _) };
        self.insert_mapping(id, DatumEntry::Page(unsafe { base.cast().as_ref() }))?;

        Ok(page)
    }

    pub fn get_constants(&self, id: &str) -> Result<&'d [Constant<'d>], DatumspaceError>
    {
        // The main difference here is that the id refers to the datumpage rather than a specific entry in it, as
        // every page only has one constant table.

        match self.mapping.get(id)
        {
            Some(&DatumEntry::Page(header)) => Ok(unsafe { header.get_page() }.constants),
            Some(_) => Err(DatumspaceError::UnexpectedDatumtype),
            None => Err(DatumspaceError::ResourceDoesntExist),
        }
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
        table_id: &str,
        table: &[TableEntry<'file>],
        functions: &[FunctionInfo<'file>],
    ) -> Result<Layout, DatumspaceError>
    {
        let mut size = size_of::<DatumPageHeader>();

        // id
        size = align_up(size + table_id.len(), align_of::<Constant>());

        // constants array
        size += table.len() * size_of::<Constant>();

        // string blobs inside constants
        for entry in table.iter()
        {
            if let TableEntry::String(s) = entry
            {
                size = align_up(size + s.len(), align_of::<u8>());
            }
        }

        // align up to Runnable before the functions array
        size = align_up(size, align_of::<Runnable>());
        size += functions.len() * size_of::<Runnable>();

        // directives + bytecode blobs per function
        for info in functions.iter()
        {
            size = align_up(size, align_of::<Directive>());
            size += info.directives.len() * size_of::<Directive>();
            size += info.code.len(); // bytecode is u8, no alignment needed
        }

        Layout::from_size_align(size, align_of::<DatumPageHeader>()).map_err(|_| DatumspaceError::AllocationFailure)
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

    #[test]
    fn load_datum_happy_path()
    {
        let mut ds = make_datumspace();
        let table = make_table("my_func");
        let code = vec![0x01, 0x02, 0x03];
        let functions = vec![make_function(0, &code)];

        let page = ds
            .load_datum("my_datum", &table, &functions)
            .expect("load_datum should succeed");

        assert_eq!(page.id, "my_datum");
        assert_eq!(page.constants.len(), table.len());
        assert_eq!(page.functions.len(), functions.len());
    }

    #[test]
    fn get_constant_returns_correct_values()
    {
        let mut ds = make_datumspace();
        let table = make_table("fn_name");
        let functions = vec![make_function(0, &[0xAB])];

        ds.load_datum("datum_a", &table, &functions).unwrap();

        // index 0 — the interned string
        assert!(matches!(
            ds.get_constants("datum_a").unwrap()[0],
            Constant::String(s) if s == "fn_name"
        ));

        // index 1 — integer primitive
        assert!(matches!(
            ds.get_constants("datum_a").unwrap()[1],
            Constant::Unsigned32(42)
        ));

        // index 2 — long primitive
        assert!(matches!(
            ds.get_constants("datum_a").unwrap()[2],
            Constant::Unsigned64(9999)
        ));
    }

    #[test]
    fn get_runnable_returns_correct_function()
    {
        let mut ds = make_datumspace();
        let table = make_table("entry");
        let code = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let functions = vec![make_function(0, &code)];

        ds.load_datum("datum_b", &table, &functions).unwrap();

        let runnable = ds.get_runnable("entry").expect("runnable should exist");
        assert_eq!(runnable.name, "entry");
        assert_eq!(runnable.bytecode, &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(runnable.maxstack, 8);
        assert_eq!(runnable.maxlocals, 4);
    }

    #[test]
    fn duplicate_datum_id_is_rejected()
    {
        let mut ds = make_datumspace();
        let table = make_table("func");
        let functions = vec![make_function(0, &[0x00])];

        ds.load_datum("same_id", &table, &functions).unwrap();

        let table2 = make_table("func2");
        let functions2 = vec![make_function(0, &[0x01])];
        let result = ds.load_datum("same_id", &table2, &functions2);

        assert!(matches!(result, Err(DatumspaceError::Duplication)));
    }

    #[test]
    fn duplicate_function_name_across_datums_is_rejected()
    {
        let mut ds = make_datumspace();

        // First datum registers "shared_fn"
        let table1 = make_table("shared_fn");
        let fns1 = vec![make_function(0, &[0x01])];
        ds.load_datum("datum_one", &table1, &fns1).unwrap();

        // Second datum also tries to register "shared_fn"
        let table2 = make_table("shared_fn");
        let fns2 = vec![make_function(0, &[0x02])];
        let result = ds.load_datum("datum_two", &table2, &fns2);

        assert!(matches!(result, Err(DatumspaceError::Duplication)));
    }

    #[test]
    fn non_string_name_index_is_rejected()
    {
        let mut ds = make_datumspace();
        // Table where index 0 is an integer, not a string
        let table = vec![TableEntry::Integer(99), TableEntry::String("real_name")];
        // name_index 0 points at the Integer — should fail
        let functions = vec![make_function(0, &[0x00])];

        let result = ds.load_datum("datum_bad_type", &table, &functions);
        assert!(matches!(result, Err(DatumspaceError::UnexpectedDatumtype)));
    }

    #[test]
    fn out_of_bounds_name_index_is_rejected()
    {
        let mut ds = make_datumspace();
        let table = make_table("some_fn"); // 5 entries, indices 0..=4
        // name_index 99 is well out of range
        let functions = vec![make_function(99, &[0x00])];

        let result = ds.load_datum("datum_oob", &table, &functions);
        assert!(matches!(result, Err(DatumspaceError::ResourceDoesntExist)));
    }
}
