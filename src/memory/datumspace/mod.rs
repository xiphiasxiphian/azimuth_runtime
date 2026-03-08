pub mod types;

use std::{alloc::Layout, collections::HashMap, ptr::NonNull};

use crate::{loader::parser::table::{Table, TableEntry}, memory::{allocators::{AllocatorError, general::GeneralAllocator}, datumspace::types::TypeInfo}};

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
}

struct Datumspace<'a>
{
    types: DatumAllocator,
    functions: DatumAllocator,
    constants: DatumAllocator,
    mapping: HashMap<&'a str, NonNull<u8>>
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
    pub fn load_constants<'file, I>(&mut self, table: I, table_id: &'file str) -> Result<&'d [TableEntry<'d>], DatumspaceError>
    where
        I: ExactSizeIterator<Item = &'file TableEntry<'file>>,
        'd: 'file
    {
        // Move the id into the datumspace for permenant storage
        let id_str: &'d str = unsafe {
            let dest = self.constants
                .copy_from_nonoverlapping(&table_id.as_bytes())
                .ok_or(DatumspaceError::AllocationFailure)?;
            str::from_utf8_unchecked(dest.as_ref())
        };

        let count = table.len();

        // Allocate space for the array of entries itself in our allocator
        let array_layout = Layout::array::<TableEntry>(count).map_err(|_| DatumspaceError::AllocationFailure)?;
        let array_ptr = self.constants
            .raw_alloc(array_layout)
            .map(|x| x.cast::<TableEntry<'d>>())
            .ok_or(DatumspaceError::AllocationFailure)?;

        // Copy data over
        for (i, entry) in table.enumerate() {
            let permanent_entry = match entry {
                // Primitives are just copied by value
                TableEntry::Integer(v) => TableEntry::Integer(*v),
                TableEntry::Long(v) => TableEntry::Long(*v),
                TableEntry::Float(v) => TableEntry::Float(*v),
                TableEntry::Double(v) => TableEntry::Double(*v),

                // Strings require a deep copy into datumspace
                TableEntry::String(file_str) => {
                    let str_bytes = file_str.as_bytes();
                    let dest_ptr = self.constants
                        .copy_from_nonoverlapping(&str_bytes)
                        .ok_or(DatumspaceError::AllocationFailure)?;

                    unsafe {
                        // Reconstitute the string slice with the Datumspace lifetime ('d)
                        let permanent_str = str::from_utf8_unchecked(dest_ptr.as_ref());

                        TableEntry::String(permanent_str)
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
            .insert(id_str, NonNull::from_ref(entries).cast())
            .map_or_else(move || Ok(entries), |_| Err(DatumspaceError::Duplication))
    }

    // pub fn push_type<'b>(&mut self, bytes: &'b [u8]) -> Result<&'a TypeInfo<'a>, DatumspaceError>
    // {
    //     // In general this means cloning in the required information for the type, as the references
    //     // will refer to the memory occupied by the file being open - as soon as the file is closed
    //     // this reference wont exist anymore.

    //     let ty = TypeInfo::from_bytes(bytes)
    //         .ok_or(DatumspaceError::InvalidStructure)
    //         .and_then(|(ty, rem)| {
    //             if rem.len() > 0 { return Err(DatumspaceError::LeftOverBytes) }
    //             Ok(ty)
    //         })?;

    //     let data = self.types.alloc(ty)
    //         .ok_or(DatumspaceError::AllocationFailure)?;

    //     assert!(data.is_aligned());

    //     // self.mapping
    //     //     .insert(unsafe { data.as_ref().id() }, data.cast())
    //     //     .map_or_else(|| Ok(unsafe { data.as_ref() }), |_| Err(DatumspaceError::Duplication))

    //     todo!()
    // }

    // pub fn get_type(&'d self, id: &str) -> Option<&'d TypeInfo<'d>>
    // {
    //     self.mapping
    //         .get(id)
    //         .map(|&slice| unsafe {
    //             let typeinfo: NonNull<TypeInfo> = slice.cast();

    //             // Ensure I haven't fucked up
    //             assert!(typeinfo.is_aligned());

    //             typeinfo.as_ref()
    //         })
    // }
}
