use binrw::{BinRead, BinResult, binread};
use bitflags::bitflags;

use crate::memory::datumspace::constant_table::{Constant, ConstantTableIndex};

const MAGIC: [u8; 8] = *b"azimuth\0";

/// 128-bit content-derived identifier for a symbol.
///
/// For functions and types, computed as: hash(namespace + name + type_descriptor), truncated to 16 bytes.
/// Being content-derived means two independent compilers targeting the same
/// source produce identical UUIDs, enabling cross-compiler linking.
///
/// For modules, Computed from the file's fully-qualified module path + version.
/// Used by the import table to verify the correct file was loaded.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId([u8; 16]);

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
    file_version: u16,
    min_runtime_version: u16,
    module_id: SymbolId,

    // module_path: &str,
    flags: FileFlags,
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
    entries: Vec<Link>
}


// Type table
// TODO

// Symbol Table

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub enum SymbolKind
{
    Function {
        // signature:
        body: Offset,
    },
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
    symbols: Vec<SymbolEntry>
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
    functions: Vec<Function>,

    #[br(temp)]
    code_length: u32,

    #[br(count = code_length)]
    bytecode: Vec<u8>,
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
    entries: Vec<DataHeader>,

    #[br(temp)]
    data_length: u32,

    #[br(count = data_length)]
    data: Vec<u8>,
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
}


// Overall File Layout

#[binread]
#[derive(Clone, Debug)]
pub struct FileLayout
{
    // Important metadata
    header: FileHeader,
    link_table: LinkTable,
    symbol_table: SymbolTable,

    // Code segment
    code_directory: CodeDirectory,

    // Data segment
}
