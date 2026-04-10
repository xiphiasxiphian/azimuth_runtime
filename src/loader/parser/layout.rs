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
    pub module_id: SymbolId,
    pub module_path: Offset,

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
    pub id: SymbolId,
    pub kind: SymbolKind,
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

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FunctionFlags: u8 {
        const ENTRYPOINT = 0b0000_0001;
    }
}

impl BinRead for FunctionFlags {
    type Args<'a> = ();

    fn read_options<R: binrw::io::Read + binrw::io::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        Ok(
            FunctionFlags::from_bits_retain(
                u8::read_options(reader, endian, args)?
            )
        )
    }
}

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct Function
{
    symbol_id: SymbolId,
    pub index: Offset,
    pub length: u32,
    pub maxlocals: u32,
    pub maxstack: u32,
    pub flags: FunctionFlags,
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
    pub length: u32,
    pub index: Offset,
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
}
