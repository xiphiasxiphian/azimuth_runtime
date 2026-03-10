pub mod types;
pub mod constant_table;
pub mod runnable;

use std::{alloc::Layout, collections::HashMap, ptr::NonNull};

use crate::{loader::parser::{function::FunctionInfo, table::{Table, TableEntry}}, memory::{allocators::{AllocatorError, general::GeneralAllocator}, datumspace::{constant_table::Constant, runnable::Runnable, types::TypeInfo}}};

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

enum DatumspaceError
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

struct Datumspace<'a>
{
    types: DatumAllocator,
    functions: DatumAllocator,
    constants: DatumAllocator,
    mapping: HashMap<&'a str, DatumEntry<'a>>
}

impl<'d> Datumspace<'d>
{
    pub fn with_capacity(min_capacity: usize) -> Result<Self, AllocatorError>
    {
        let adjusted_capacity = min_capacity.next_power_of_two();
        let individual_capacity = adjusted_capacity / 4; // For now just split in half. This might be revisited at some point

        Ok(
            Self {
                types: DatumAllocator::with_capacity(individual_capacity)?,
                functions: DatumAllocator::with_capacity(individual_capacity * 2)?,
                constants: DatumAllocator::with_capacity(individual_capacity)?,
                mapping: HashMap::new(),
            }
        )
    }

    /// Takes the transient Table (tied to the file) and deep-copies
    /// the data into the Datumspace's custom allocator.
    pub fn load_constants<'file, I>(&mut self, table: I, table_id: &'file str) -> Result<&'d [Constant<'d>], DatumspaceError>
    where
        I: ExactSizeIterator<Item = &'file TableEntry<'file>>,
        'd: 'file
    {
        // Move the id into the datumspace for permenant storage
        let id_str: &'d str = unsafe {
            let dest = self.constants
                .copy_bytes(&table_id.as_bytes())
                .ok_or(DatumspaceError::AllocationFailure)?;
            str::from_utf8_unchecked(dest.as_ref())
        };

        let count = table.len();

        // Allocate space for the array of entries itself in our allocator
        let array_layout = Layout::array::<TableEntry>(count).map_err(|_| DatumspaceError::AllocationFailure)?;
        let array_ptr = self.constants
            .raw_alloc(array_layout)
            .map(|x| x.cast::<Constant<'d>>())
            .ok_or(DatumspaceError::AllocationFailure)?;

        // Copy data over
        for (i, entry) in table.enumerate() {
            let permanent_entry = match entry {
                // Primitives are just copied by value
                TableEntry::Integer(v) => Constant::Unsigned32(*v),
                TableEntry::Long(v) => Constant::Unsigned64(*v),
                TableEntry::Float(v) => Constant::Float32(*v),
                TableEntry::Double(v) => Constant::Float64(*v),

                // Strings require a deep copy into datumspace
                TableEntry::String(file_str) => {
                    let str_bytes = file_str.as_bytes();
                    let dest_ptr = self.constants
                        .copy_bytes(&str_bytes)
                        .ok_or(DatumspaceError::AllocationFailure)?;

                    unsafe {
                        // Reconstitute the string slice with the Datumspace lifetime ('d)
                        let permanent_str = str::from_utf8_unchecked(dest_ptr.as_ref());
                        Constant::String(permanent_str)
                    }
                }
            };

            // Write the permanent entry into our allocated array
            unsafe {
                array_ptr.as_ptr().add(i).write(permanent_entry);
            }
        }

        // Store the slice referencing our custom allocator
        let entries = unsafe {
            std::slice::from_raw_parts(array_ptr.as_ptr(), count)
        };

        self.mapping
            .insert(id_str, DatumEntry::ConstantTable(entries))
            .map_or_else(|| Ok(entries), |_| Err(DatumspaceError::Duplication))
    }

    pub fn push_function<'file>(&'d mut self, table_id: &str, function: &'file FunctionInfo<'file>) -> Result<&'d Runnable<'d>, DatumspaceError>
    {
        let name = self.get_constant(table_id, function.name_index)
            .and_then(|x| match x {
                &Constant::String(nm) => Ok(nm),
                _ => Err(DatumspaceError::UnexpectedDatumtype)
            })?;


        let runnable = Runnable::from_parsed_data(&mut self.functions, &function.directives, function.code)?;

        self.mapping
            .insert(name, DatumEntry::Function(runnable))
            .map_or_else(|| Ok(runnable), |_| Err(DatumspaceError::Duplication))
    }

    pub fn get_constant(&self, id: &str, index: usize) -> Result<&'d Constant<'d>, DatumspaceError>
    {
        match self.mapping.get(id)
        {
            Some(&DatumEntry::ConstantTable(consts)) => {
                consts.get(index).ok_or(DatumspaceError::ResourceDoesntExist)
            }
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
}
