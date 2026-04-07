use binrw::{BinRead, BinResult, binread};
use bitflags::bitflags;

use crate::loader::SymbolId;

type Offset = u32;


bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileFlags: u8 {
        const HAS_DEBUG = 0b0000_0001;
    }
}

impl BinRead for FileFlags {
    type Args<'a> = ();

    fn read_options<R: binrw::io::Read + binrw::io::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        Ok(
            FileFlags::from_bits_retain(
                u8::read_options(reader, endian, args)?
            )
        )
    }
}

#[binread]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[br(little)]
pub struct FileHeader
{
    pub file_version: u16,
    pub min_runtime_version: u16,
    pub module_id: SymbolId,

    // module_path: &str,
    pub flags: FileFlags,
}


// Import Table

#[binread]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[br(little)]
pub struct Link
{
    index: u16,
    module_id: SymbolId,
    module_path: Offset,

    // lazy: bool
}

#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct LinkTable
{
    #[br(temp)]
    count: u32,

    #[br(count = count)]
    pub entries: Vec<Link>
}


// Type table
// TODO

// Symbol Table

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub enum SymbolKind
{
    #[br(magic = 0u8)]
    Function {
        // signature:
        body: Offset,
    },

    #[br(magic = 1u8)]
    Type {
        // typeref
    }
}

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct SymbolEntry
{
    id: SymbolId,
    qualified_name: Offset,
    kind: SymbolKind,
    // flags?
}

#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct SymbolTable
{
    #[br(temp)]
    count: u32,

    #[br(count = count)]
    pub symbols: Vec<SymbolEntry>
}


// Code blocks

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct FunctionHeader
{
    length: u32,
    maxlocals: u32,
    maxstack: u32,
}

impl FunctionHeader
{
    pub unsafe fn get_code(&self) -> &[u8]
    {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self).add(1) as *const u8,
                self.length.try_into().expect("Running on sub 32-bit architecture"))
        }
    }

    pub unsafe fn next(&self) -> &FunctionHeader
    {
        let length: usize = self.length.try_into().expect("Running on sub 32-bit architecture");
        unsafe {
            &*((self as *const Self).byte_add(size_of::<Self>() + length))
        }
    }
}

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct Function
{
    symbol_id: SymbolId,
    offset: Offset,
    // flags?
}

#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct CodeDirectory
{
    #[br(temp)]
    func_count: u32,

    #[br(count = func_count)]
    pub functions: Vec<Function>,

    #[br(temp)]
    code_length: u32,

    #[br(count = code_length)]
    pub bytecode: Vec<u8>,
}

impl CodeDirectory
{
    pub fn bytecode_size(&self) -> usize
    {
        self.bytecode.len()
    }

    pub fn function_count(&self) -> usize
    {
        self.functions.len()
    }
}

// Data

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct DataHeader
{
    length: u32,
    offset: Offset,
    // type ref maybe?
    // any flags?
}

#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct DataDirectory
{
    #[br(temp)]
    entry_count: u32,

    #[br(count = entry_count)]
    pub entries: Vec<DataHeader>,

    #[br(temp)]
    data_length: u32,

    #[br(count = data_length)]
    pub data: Vec<u8>,
}

impl DataDirectory
{
    pub fn get(&self, offset: Offset) -> Option<&[u8]>
    {
        let entry = self.entries.get(offset as usize)?;

        let start = entry.offset as usize;
        let end = start + entry.length as usize;

        self.data.get(start..end)
    }

    pub fn data_byte_size(&self) -> usize
    {
        self.data.len()
    }

    pub fn entries_byte_size(&self) -> usize
    {
        self.entries.len() * size_of::<DataHeader>()
    }
}


// Overall File Layout

#[binread]
#[derive(Clone, Debug)]
#[br(magic = b"azimuth\0")]
#[br(little)]
pub struct FileLayout
{
    // Important metadata
    pub header: FileHeader,
    pub link_table: LinkTable,
    pub symbol_table: SymbolTable,

    // Code segment
    pub code_directory: CodeDirectory,

    // Data segment
    pub data_directory: DataDirectory,
}

#[cfg(test)]
mod tests {
    use super::*;
    use binrw::{BinRead, io::Cursor};

    #[test]
    fn test_file_flags_parsing() {
        // Test parsing the custom bitflags implementation
        let mut cursor = Cursor::new(vec![0b0000_0001]); // HAS_DEBUG
        let flags = FileFlags::read_le(&mut cursor).unwrap();
        assert_eq!(flags, FileFlags::HAS_DEBUG);

        let mut cursor_empty = Cursor::new(vec![0b0000_0000]);
        let flags_empty = FileFlags::read_le(&mut cursor_empty).unwrap();
        assert!(flags_empty.is_empty());
    }

    #[test]
    fn test_file_layout_parsing() {
        let mut data = Vec::new();

        // --- Magic ---
        data.extend_from_slice(b"azimuth\0");

        // --- FileHeader ---
        data.extend_from_slice(&1u16.to_le_bytes()); // file_version = 1
        data.extend_from_slice(&1u16.to_le_bytes()); // min_runtime_version = 1
        data.extend_from_slice(&[0u8; 16]);          // module_id (SymbolId)
        data.push(1);                                // flags (FileFlags::HAS_DEBUG)

        // --- LinkTable ---
        data.extend_from_slice(&0u32.to_le_bytes()); // count = 0

        // --- SymbolTable ---
        data.extend_from_slice(&0u32.to_le_bytes()); // count = 0

        // --- CodeDirectory ---
        data.extend_from_slice(&0u32.to_le_bytes()); // func_count = 0
        data.extend_from_slice(&0u32.to_le_bytes()); // code_length = 0

        // --- DataDirectory ---
        data.extend_from_slice(&0u32.to_le_bytes()); // entry_count = 0
        data.extend_from_slice(&0u32.to_le_bytes()); // data_length = 0

        // Parse the constructed bytes
        let mut cursor = Cursor::new(data);
        let layout = FileLayout::read_le(&mut cursor).expect("Failed to parse valid minimal FileLayout");

        // Verify the parsed data
        assert_eq!(layout.header.file_version, 1);
        assert_eq!(layout.header.flags, FileFlags::HAS_DEBUG);
        assert_eq!(layout.link_table.entries.len(), 0);
        assert_eq!(layout.symbol_table.symbols.len(), 0);
    }

    #[test]
    fn test_data_directory_get() {
        // Instantiate manually to test the `get` slicing logic independently of binrw
        let dir = DataDirectory {
            entries: vec![
                DataHeader { length: 4, offset: 0 },
                DataHeader { length: 2, offset: 4 },
                DataHeader { length: 3, offset: 10 }, // Purposely out of bounds to test safety
            ],
            data: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };

        // Valid retrievals
        assert_eq!(dir.get(0).unwrap(), &[10, 20, 30, 40]);
        assert_eq!(dir.get(1).unwrap(), &[50, 60]);

        // Invalid index
        assert_eq!(dir.get(99), None);

        // Valid index, but slice out of bounds of the `data` array
        assert_eq!(dir.get(2), None);
    }

    #[test]
    fn test_function_header_pointers() {
        // We use a u32 array to guarantee 4-byte memory alignment, which is critical
        // when casting raw pointers back into structs like FunctionHeader.
        let memory_block: [u32; 7] = [
            4,          // [0] Header 1: length (4 bytes of code)
            2,          // [1] Header 1: maxlocals
            2,          // [2] Header 1: maxstack
            0xEFBEADDE, // [3] Code for Header 1 (4 bytes: DE AD BE EF in little-endian)
            8,          // [4] Header 2: length (8 bytes of code)
            0,          // [5] Header 2: maxlocals
            0,          // [6] Header 2: maxstack
        ];

        unsafe {
            // Get a pointer to the start of our memory block
            let ptr = memory_block.as_ptr() as *const FunctionHeader;
            let header1 = &*ptr;

            // Test get_code()
            let code = header1.get_code();
            assert_eq!(code.len(), 4);
            // On little endian systems, 0xEFBEADDE is represented as [0xDE, 0xAD, 0xBE, 0xEF]
            assert_eq!(code, &[0xDE, 0xAD, 0xBE, 0xEF]);

            // Test next() pointer math
            let header2 = header1.next();
            assert_eq!(header2.length, 8);
            assert_eq!(header2.maxlocals, 0);
        }
    }
}
