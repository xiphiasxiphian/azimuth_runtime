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

impl BinRead for FileFlags
{
    type Args<'a> = ();

    fn read_options<R: binrw::io::Read + binrw::io::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self>
    {
        Ok(FileFlags::from_bits_retain(u8::read_options(reader, endian, args)?))
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
    pub entries: Vec<Link>,
}

// Symbol Table

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub enum SymbolKind
{
    #[br(magic = 0_u8)]
    Function
    {
        // signature:
        body: Offset,
    },

    #[br(magic = 1_u8)]
    Type {
        type_index: u32,
    },
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
    pub symbols: Vec<SymbolEntry>,
}

// Types

// Pure value types: stored inline, pointer-free, never GC-traced.
//
// `String` is intentionally absent. A string is a heap object (ObjRef) and is
// represented as `TypeSignature::String` so the layout engine always traces it
// without any special-case logic in the Scalar branch.

#[binread]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[br(little)]
pub enum ScalarTag
{
    #[br(magic = 0x0_u8)]
    Integer32,
    #[br(magic = 0x1_u8)]
    Integer64,
    #[br(magic = 0x2_u8)]
    Float32,
    #[br(magic = 0x3_u8)]
    Float64,
}

// Describes what a field, local variable, or parameter holds and how the GC
// must treat it. The four variants map directly to GC policy:
//
//   Scalar      -> never a GC root (stored by value, no heap pointer)
//   String      -> always a GC root (ObjRef to heap-allocated UTF-8 string)
//   ValueType   -> never a GC root (inline layout; loader validates no nested
//                 String/Reference fields anywhere in the transitive closure)
//   Reference   -> always a GC root (ObjRef to a heap-allocated UserDefinedType)
//
// Both ValueType and Reference carry a `type_index` (0-based into
// `TypeDirectory::types`) rather than a SymbolId. This keeps the binary
// compact (4 bytes vs 16) and makes runtime lookup O(1). The loader resolves
// cross-module SymbolIds from the link table and rewrites them to local indices
// before handing the module to the runtime.

#[binread]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[br(little)]
pub enum TypeSignature
{
    /// Stored inline. Never a GC root.
    #[br(magic = 0x00_u8)]
    Scalar(ScalarTag),

    /// Heap-allocated UTF-8 string (ObjRef). Always a GC root. String
    /// constants available to bytecode are stored in the data directory as
    /// `ConstantKind::StringUtf8` entries and pushed via `PUSH_CONST`.
    #[br(magic = 0x01_u8)]
    String,

    /// Inline (value-type) struct or enum embedded directly in the parent
    /// allocation. Never a GC root.
    ///
    /// **Loader invariant**: every field in the transitive closure of this
    /// type MUST be `Scalar`. A value type that transitively contains a
    /// `String` or `Reference` field is malformed and must be rejected.
    #[br(magic = 0x02_u8)]
    ValueType
    {
        type_index: u32
    },

    /// Heap-allocated object (ObjRef). Always a GC root. The runtime layout
    /// engine adds this field's byte offset to `Traceable::references()`.
    #[br(magic = 0x03_u8)]
    Reference
    {
        type_index: u32
    },
}

// type directory

/// A single field in a struct or enum variant.
///
/// `name` is a data directory index. The entry it points to MUST have
/// `ConstantKind::StringUtf8`. This means field names share the same constant
/// pool as runtime string literals: the linker or compiler writes each unique
/// name once and both metadata and bytecode reference it by index.
#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct FieldDef
{
    pub name: u32,
    pub signature: TypeSignature,
}

/// A product type: an ordered, named collection of fields laid out sequentially
/// in the heap allocation (after the `ObjectHeader`).
#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct StructDef
{
    pub symbol_id: SymbolId,

    #[br(temp)]
    field_count: u16,

    #[br(count = field_count)]
    pub fields: Vec<FieldDef>,
}

/// One arm of an algebraic enum.
///
/// `tag` is the discriminant value written into every live allocation of this
/// variant. It is NOT implied by the variant's position in the `variants` vec:
/// using an explicit tag means variants can be added, removed, or reordered
/// without changing the wire value of any existing variant, and non-contiguous
/// discriminant ranges are supported.
///
/// `name` follows the same convention as `FieldDef::name` (data directory
/// index of a `ConstantKind::StringUtf8` entry).
#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct EnumVariant
{
    pub name: u32,
    pub tag: u32,

    #[br(temp)]
    field_count: u16,

    #[br(count = field_count)]
    pub fields: Vec<FieldDef>,
}

/// A sum type: exactly one variant is live at runtime.
///
/// A heap allocation of an enum looks like:
///
///   [ObjectHeader][discriminant: u32][padding][variant payload]
///
/// The discriminant is matched against `EnumVariant::tag` (not position) to
/// find the active variant. `Traceable::references()` reads the discriminant
/// and returns only the active variant's GC roots, so no dead pointer slots
/// are ever traced.
#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct EnumDef
{
    pub symbol_id: SymbolId,

    #[br(temp)]
    variant_count: u16,

    #[br(count = variant_count)]
    pub variants: Vec<EnumVariant>,
}

#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub enum UserDefinedType
{
    #[br(magic = 0x00_u8)]
    Struct(StructDef),

    #[br(magic = 0x01_u8)]
    Enum(EnumDef),

    /// A stub representing an imported ValueType.
    /// `link_index` points into the local LinkTable to identify
    /// the target module and the exported SymbolId.
    #[br(magic = 0x02_u8)]
    Imported
    {
        local_id: SymbolId,
        link_index: u32,
        target_id: SymbolId,
    },
}

/// All user-defined types in the module, in declaration order.
///
/// A `UserDefinedType` at index `i` is reachable via:
///   - `TypeSignature::ValueType { type_index: i }` / `Reference { type_index: i }`
///   - `SymbolKind::Type { type_index: i }` in the symbol table
///   - `ObjectHeader::type_index = i` in every live heap object at runtime
///
/// All three use the same 0-based index, so the GC can dispatch to the correct
/// layout with a single bounds-checked array access.
#[binread]
#[derive(Clone, Debug)]
#[br(little)]
pub struct TypeDirectory
{
    #[br(temp)]
    count: u32,

    #[br(count = count)]
    pub types: Vec<UserDefinedType>,
}

impl TypeDirectory
{
    pub fn type_count(&self) -> usize
    {
        self.types.len()
    }
}

// Code blocks

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FunctionFlags: u8 {
        const ENTRYPOINT = 0b0000_0001;
    }
}

impl BinRead for FunctionFlags
{
    type Args<'a> = ();

    fn read_options<R: binrw::io::Read + binrw::io::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self>
    {
        Ok(FunctionFlags::from_bits_retain(u8::read_options(reader, endian, args)?))
    }
}

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct Function
{
    pub symbol_id: SymbolId,
    pub index: Offset,
    pub length: u32,
    pub maxlocals: u32,
    pub maxstack: u32,
    pub param_count: u8,
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

// Describes the layout of a constant in the data directory.
// This deliberately omits `Reference`, guaranteeing that constants
// can never require heap allocation for user-defined types.
#[binread]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[br(little)]
pub enum ConstantSignature
{
    #[br(magic = 0x00_u8)]
    Scalar(ScalarTag),

    #[br(magic = 0x01_u8)]
    String,

    #[br(magic = 0x02_u8)]
    ValueType
    {
        type_index: u32
    },
}

#[binread]
#[derive(Clone, Copy, Debug)]
#[br(little)]
pub struct DataHeader
{
    pub length: u32,
    pub index: Offset,
    pub signature: ConstantSignature,
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

    // Type segment
    pub type_directory: TypeDirectory,

    // Data segment
    pub data_directory: DataDirectory,
}

#[cfg(test)]
mod tests
{
    use binrw::{BinRead, io::Cursor};

    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn dummy_symbol_id(byte: u8) -> [u8; 16]
    {
        [byte; 16]
    }

    /// Builds a minimal but complete FileLayout byte buffer.
    struct FileBuilder
    {
        data: Vec<u8>,
    }

    impl FileBuilder
    {
        fn new() -> Self
        {
            let mut b = Self { data: Vec::new() };
            b.data.extend_from_slice(b"azimuth\0");
            b
        }

        fn header(mut self, file_version: u16, min_runtime: u16, module_id: [u8; 16], flags: u8) -> Self
        {
            self.data.extend_from_slice(&file_version.to_le_bytes());
            self.data.extend_from_slice(&min_runtime.to_le_bytes());
            self.data.extend_from_slice(&module_id);
            self.data.push(flags);
            self
        }

        fn link_table(mut self, links: &[(u16, [u8; 16], u32)]) -> Self
        {
            self.data.extend_from_slice(&(links.len() as u32).to_le_bytes());
            for (index, module_id, module_path) in links
            {
                self.data.extend_from_slice(&index.to_le_bytes());
                self.data.extend_from_slice(module_id);
                self.data.extend_from_slice(&module_path.to_le_bytes());
            }
            self
        }

        /// Each symbol: (id: [u8;16], kind_tag: u8, extra: Option<u32> for Function body)
        fn symbol_table(mut self, symbols: &[([u8; 16], u8, Option<u32>)]) -> Self
        {
            self.data.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
            for (id, kind, extra) in symbols
            {
                self.data.extend_from_slice(id);
                self.data.push(*kind);
                if let Some(body) = extra
                {
                    self.data.extend_from_slice(&body.to_le_bytes());
                }
            }
            self
        }

        /// funcs: (symbol_id, index, length, maxlocals, maxstack, param_count, flags)
        ///
        /// NOTE: `param_count` sits between `maxstack` and `flags` in the binary
        /// layout, matching the definition of `Function` exactly.
        fn code_directory(mut self, funcs: &[([u8; 16], u32, u32, u32, u32, u8, u8)], bytecode: &[u8]) -> Self
        {
            self.data.extend_from_slice(&(funcs.len() as u32).to_le_bytes());
            for (sym, idx, len, locals, stack, param_count, flags) in funcs
            {
                self.data.extend_from_slice(sym);
                self.data.extend_from_slice(&idx.to_le_bytes());
                self.data.extend_from_slice(&len.to_le_bytes());
                self.data.extend_from_slice(&locals.to_le_bytes());
                self.data.extend_from_slice(&stack.to_le_bytes());
                self.data.push(*param_count);
                self.data.push(*flags);
            }
            self.data.extend_from_slice(&(bytecode.len() as u32).to_le_bytes());
            self.data.extend_from_slice(bytecode);
            self
        }

        /// entries: (length, index, signature)
        ///
        /// NOTE: `signature` is the third field of `DataHeader`. Because it is an enum,
        /// we manually serialize it into bytes exactly as `binrw` expects to parse it.
        fn data_directory(mut self, entries: &[(u32, u32, ConstantSignature)], data: &[u8]) -> Self
        {
            self.data.extend_from_slice(&(entries.len() as u32).to_le_bytes());

            for (length, index, signature) in entries
            {
                self.data.extend_from_slice(&length.to_le_bytes());
                self.data.extend_from_slice(&index.to_le_bytes());

                // Serialize the ConstantSignature enum
                match signature
                {
                    ConstantSignature::Scalar(scalar_tag) =>
                    {
                        self.data.push(0x00); // ConstantSignature::Scalar magic byte
                        self.data.push(*scalar_tag as u8); // ScalarTag magic byte
                    }
                    ConstantSignature::String =>
                    {
                        self.data.push(0x01); // ConstantSignature::String magic byte
                    }
                    ConstantSignature::ValueType { type_index } =>
                    {
                        self.data.push(0x02); // ConstantSignature::ValueType magic byte
                        self.data.extend_from_slice(&type_index.to_le_bytes());
                    }
                }
            }

            self.data.extend_from_slice(&(data.len() as u32).to_le_bytes());
            self.data.extend_from_slice(data);
            self
        }

        fn build(self) -> Vec<u8>
        {
            self.data
        }
    }

    /// Builds the smallest possible valid `FileLayout` buffer with the given
    /// header flags byte and no links, symbols, functions, or data entries.
    fn minimal_layout(flags: u8) -> Vec<u8>
    {
        FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), flags)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build()
    }

    // ── FileFlags ─────────────────────────────────────────────────────────────

    #[test]
    fn file_flags_has_debug_set()
    {
        let mut c = Cursor::new(vec![0b0000_0001]);
        let f = FileFlags::read_le(&mut c).unwrap();
        assert!(f.contains(FileFlags::HAS_DEBUG));
    }

    #[test]
    fn file_flags_empty()
    {
        let mut c = Cursor::new(vec![0u8]);
        let f = FileFlags::read_le(&mut c).unwrap();
        assert!(f.is_empty());
    }

    #[test]
    fn file_flags_unknown_bits_retained()
    {
        // Bits that are not named flags must not be silently dropped.
        let mut c = Cursor::new(vec![0b1111_1110]);
        let f = FileFlags::read_le(&mut c).unwrap();
        assert_eq!(f.bits(), 0b1111_1110);
        assert!(!f.contains(FileFlags::HAS_DEBUG));
    }

    #[test]
    fn file_flags_all_bits_set()
    {
        let mut c = Cursor::new(vec![0xFF]);
        let f = FileFlags::read_le(&mut c).unwrap();
        assert!(f.contains(FileFlags::HAS_DEBUG));
        assert_eq!(f.bits(), 0xFF);
    }

    // ── FunctionFlags ─────────────────────────────────────────────────────────

    #[test]
    fn function_flags_entrypoint_set()
    {
        let mut c = Cursor::new(vec![0b0000_0001]);
        let f = FunctionFlags::read_le(&mut c).unwrap();
        assert!(f.contains(FunctionFlags::ENTRYPOINT));
    }

    #[test]
    fn function_flags_empty()
    {
        let mut c = Cursor::new(vec![0u8]);
        let f = FunctionFlags::read_le(&mut c).unwrap();
        assert!(f.is_empty());
    }

    #[test]
    fn function_flags_unknown_bits_retained()
    {
        let mut c = Cursor::new(vec![0b1111_1110]);
        let f = FunctionFlags::read_le(&mut c).unwrap();
        assert_eq!(f.bits(), 0b1111_1110);
        assert!(!f.contains(FunctionFlags::ENTRYPOINT));
    }

    // ScalarTag

    #[test]
    fn scalar_tag_all_valid_discriminants_parse()
    {
        let cases: &[(u8, ScalarTag)] = &[
            (0x0, ScalarTag::Integer32),
            (0x1, ScalarTag::Integer64),
            (0x2, ScalarTag::Float32),
            (0x3, ScalarTag::Float64),
        ];

        for (byte, expected) in cases
        {
            let mut c = Cursor::new(vec![*byte]);
            let tag = ScalarTag::read_le(&mut c).unwrap();
            assert_eq!(tag, *expected, "byte 0x{byte:02X} should parse as {expected:?}");
        }
    }

    #[test]
    fn scalar_tag_invalid_discriminant_returns_error()
    {
        // 0x04 is not a defined ScalarTag variant.
        let mut c = Cursor::new(vec![0x04]);
        assert!(ScalarTag::read_le(&mut c).is_err());
    }

    #[test]
    fn scalar_tag_max_byte_returns_error()
    {
        let mut c = Cursor::new(vec![0xFF]);
        assert!(ScalarTag::read_le(&mut c).is_err());
    }

    // ── FileHeader ────────────────────────────────────────────────────────────

    #[test]
    fn file_header_versions_roundtrip()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.header.file_version, 1);
        assert_eq!(layout.header.min_runtime_version, 1);
    }

    #[test]
    fn file_header_large_version_numbers()
    {
        let bytes = FileBuilder::new()
            .header(0xFFFF, 0xBEEF, dummy_symbol_id(7), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.header.file_version, 0xFFFF);
        assert_eq!(layout.header.min_runtime_version, 0xBEEF);
    }

    #[test]
    fn file_header_module_id_preserved()
    {
        let id = dummy_symbol_id(0xAB);
        let bytes = FileBuilder::new()
            .header(1, 1, id, 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(format!("{:?}", layout.header.module_id), format!("{:?}", SymbolId(id)));
    }

    #[test]
    fn file_header_flags_debug_set()
    {
        let bytes = minimal_layout(0b0000_0001);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.header.flags, FileFlags::HAS_DEBUG);
    }

    #[test]
    fn file_header_flags_empty()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert!(layout.header.flags.is_empty());
    }

    // ── Magic bytes ───────────────────────────────────────────────────────────

    #[test]
    fn wrong_magic_returns_error()
    {
        let mut bad = Vec::new();
        bad.extend_from_slice(b"BADMAGIC"); // wrong 8 bytes
        bad.extend_from_slice(&minimal_layout(0)[8..]); // rest is valid
        assert!(FileLayout::read_le(&mut Cursor::new(bad)).is_err());
    }

    #[test]
    fn truncated_magic_returns_error()
    {
        // "azimuth" is 7 bytes; the null terminator is required.
        let bytes = b"azimuth".to_vec();
        assert!(FileLayout::read_le(&mut Cursor::new(bytes)).is_err());
    }

    // ── LinkTable ─────────────────────────────────────────────────────────────

    #[test]
    fn link_table_empty()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.link_table.entries.len(), 0);
    }

    #[test]
    fn link_table_single_entry()
    {
        let link = (42u16, dummy_symbol_id(1), 0xDEAD_BEEFu32);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[link])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.link_table.entries.len(), 1);
        assert_eq!(layout.link_table.entries[0].module_path, 0xDEAD_BEEF);
    }

    #[test]
    fn link_table_multiple_entries()
    {
        let links = [
            (0u16, dummy_symbol_id(1), 100u32),
            (1u16, dummy_symbol_id(2), 200u32),
            (2u16, dummy_symbol_id(3), 300u32),
        ];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&links)
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.link_table.entries.len(), 3);
        assert_eq!(layout.link_table.entries[0].module_path, 100);
        assert_eq!(layout.link_table.entries[1].module_path, 200);
        assert_eq!(layout.link_table.entries[2].module_path, 300);
    }

    #[test]
    fn link_table_module_ids_distinct()
    {
        let links = [(0u16, dummy_symbol_id(0xAA), 0u32), (1u16, dummy_symbol_id(0xBB), 0u32)];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&links)
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_ne!(
            format!("{:?}", layout.link_table.entries[0].module_id),
            format!("{:?}", layout.link_table.entries[1].module_id),
        );
    }

    // ── SymbolTable ───────────────────────────────────────────────────────────

    #[test]
    fn symbol_table_empty()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.symbol_table.symbols.len(), 0);
    }

    #[test]
    fn symbol_table_function_kind()
    {
        // kind tag 0 = Function, followed by body: Offset (u32)
        let sym = (dummy_symbol_id(1), 0u8, Some(0x1234_5678u32));
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[sym])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.symbol_table.symbols.len(), 1);
        match layout.symbol_table.symbols[0].kind
        {
            SymbolKind::Function { body } => assert_eq!(body, 0x1234_5678),
            other => panic!("expected Function, got {:?}", other),
        }
    }

    #[test]
    fn symbol_table_type_kind()
    {
        // kind tag 1 = Type (no extra fields)
        let sym = (dummy_symbol_id(2), 1u8, None);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[sym])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.symbol_table.symbols.len(), 1);
        assert!(matches!(layout.symbol_table.symbols[0].kind, SymbolKind::Type { type_index: 0 }));
    }

    #[test]
    fn symbol_table_mixed_kinds()
    {
        let syms = [
            (dummy_symbol_id(1), 0u8, Some(10u32)),
            (dummy_symbol_id(2), 1u8, None),
            (dummy_symbol_id(3), 0u8, Some(20u32)),
        ];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&syms)
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.symbol_table.symbols.len(), 3);
        assert!(matches!(
            layout.symbol_table.symbols[0].kind,
            SymbolKind::Function { body: 10 }
        ));
        assert!(matches!(layout.symbol_table.symbols[1].kind, SymbolKind::Type { type_index: 0 }));
        assert!(matches!(
            layout.symbol_table.symbols[2].kind,
            SymbolKind::Function { body: 20 }
        ));
    }

    #[test]
    fn symbol_table_invalid_kind_tag_returns_error()
    {
        // Tag 0xFF is not a valid SymbolKind discriminant.
        let sym = (dummy_symbol_id(1), 0xFFu8, None);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[sym])
            .code_directory(&[], &[])
            .data_directory(&[], &[])
            .build();
        assert!(FileLayout::read_le(&mut Cursor::new(bytes)).is_err());
    }

    // ── CodeDirectory ─────────────────────────────────────────────────────────

    #[test]
    fn code_directory_empty()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.function_count(), 0);
        assert_eq!(layout.code_directory.bytecode_size(), 0);
    }

    #[test]
    fn code_directory_single_function_no_entrypoint()
    {
        // (symbol_id, index, length, maxlocals, maxstack, param_count, flags)
        let func = (dummy_symbol_id(1), 0u32, 10u32, 4u32, 8u32, 3u8, 0u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[0xDE, 0xAD, 0xBE, 0xEF])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.function_count(), 1);
        let f = &layout.code_directory.functions[0];
        assert_eq!(f.index, 0);
        assert_eq!(f.length, 10);
        assert_eq!(f.maxlocals, 4);
        assert_eq!(f.maxstack, 8);
        assert_eq!(f.param_count, 3);
        assert!(f.flags.is_empty());
    }

    #[test]
    fn code_directory_function_entrypoint_flag()
    {
        let func = (dummy_symbol_id(1), 0u32, 5u32, 2u32, 4u32, 0u8, 0b0000_0001u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        let f = &layout.code_directory.functions[0];
        assert!(f.flags.contains(FunctionFlags::ENTRYPOINT));
    }

    #[test]
    fn code_directory_param_count_zero()
    {
        let func = (dummy_symbol_id(1), 0u32, 0u32, 0u32, 0u32, 0u8, 0u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.functions[0].param_count, 0);
    }

    #[test]
    fn code_directory_param_count_max()
    {
        let func = (dummy_symbol_id(1), 0u32, 0u32, 0u32, 0u32, u8::MAX, 0u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.functions[0].param_count, u8::MAX);
    }

    #[test]
    fn code_directory_param_count_preserved_across_multiple_functions()
    {
        // Each function carries a distinct param_count; make sure values aren't
        // crossed between adjacent entries.
        let funcs = [
            (dummy_symbol_id(1), 0u32, 2u32, 0u32, 0u32, 1u8, 0u8),
            (dummy_symbol_id(2), 2u32, 3u32, 0u32, 0u32, 4u8, 0u8),
            (dummy_symbol_id(3), 5u32, 1u32, 0u32, 0u32, 0u8, 0u8),
        ];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&funcs, &[0u8; 6])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.functions[0].param_count, 1);
        assert_eq!(layout.code_directory.functions[1].param_count, 4);
        assert_eq!(layout.code_directory.functions[2].param_count, 0);
    }

    #[test]
    fn code_directory_bytecode_preserved()
    {
        let bytecode = vec![0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &bytecode)
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.bytecode, bytecode);
        assert_eq!(layout.code_directory.bytecode_size(), 6);
    }

    #[test]
    fn code_directory_multiple_functions()
    {
        let funcs = [
            (dummy_symbol_id(1), 0u32, 3u32, 1u32, 2u32, 2u8, 0u8),
            (dummy_symbol_id(2), 3u32, 5u32, 2u32, 4u32, 0u8, 0b0000_0001u8),
        ];
        let bytecode = vec![0xAA; 8];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&funcs, &bytecode)
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.function_count(), 2);
        assert!(
            !layout.code_directory.functions[0]
                .flags
                .contains(FunctionFlags::ENTRYPOINT)
        );
        assert!(
            layout.code_directory.functions[1]
                .flags
                .contains(FunctionFlags::ENTRYPOINT)
        );
        assert_eq!(layout.code_directory.functions[1].index, 3);
    }

    #[test]
    fn code_directory_function_index_and_length()
    {
        // Verify index + length fields parse correctly with large values.
        let func = (dummy_symbol_id(1), 0x0000_FFFFu32, 0xFFFF_0000u32, 0u32, 0u32, 0u8, 0u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        let f = &layout.code_directory.functions[0];
        assert_eq!(f.index, 0x0000_FFFF);
        assert_eq!(f.length, 0xFFFF_0000);
    }

    #[test]
    fn code_directory_max_locals_and_stack()
    {
        let func = (dummy_symbol_id(1), 0u32, 0u32, u32::MAX, u32::MAX, 0u8, 0u8);
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[func], &[])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        let f = &layout.code_directory.functions[0];
        assert_eq!(f.maxlocals, u32::MAX);
        assert_eq!(f.maxstack, u32::MAX);
    }

    // ── DataDirectory ─────────────────────────────────────────────────────────

    #[test]
    fn data_directory_empty()
    {
        let bytes = minimal_layout(0);
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.data_directory.data_byte_size(), 0);
        assert_eq!(layout.data_directory.entries_byte_size(), 0);
        assert_eq!(layout.data_directory.entries.len(), 0);
    }

    #[test]
    fn data_directory_single_entry()
    {
        // (length, index, signature)
        let entry = (16u32, 0u32, ConstantSignature::Scalar(ScalarTag::Integer32));
        let payload = vec![0xBE; 16];

        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[entry], &payload)
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(layout.data_directory.entries.len(), 1);
        assert_eq!(layout.data_directory.entries[0].length, 16);
        assert_eq!(layout.data_directory.entries[0].index, 0);
        assert!(matches!(
            layout.data_directory.entries[0].signature,
            ConstantSignature::Scalar(ScalarTag::Integer32)
        ));
        assert_eq!(layout.data_directory.data, payload);
        assert_eq!(layout.data_directory.data_byte_size(), 16);
    }

    #[test]
    fn data_directory_multiple_entries()
    {
        let entries = [
            (8u32, 0u32, ConstantSignature::Scalar(ScalarTag::Integer32)),
            (4u32, 8u32, ConstantSignature::Scalar(ScalarTag::Float32)),
            (16u32, 12u32, ConstantSignature::String),
        ];
        let payload: Vec<u8> = (0..28).collect();

        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&entries, &payload)
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(layout.data_directory.entries.len(), 3);
        assert_eq!(layout.data_directory.entries[0].length, 8);
        assert_eq!(layout.data_directory.entries[1].index, 8);
        assert_eq!(layout.data_directory.entries[2].length, 16);
        assert_eq!(layout.data_directory.data_byte_size(), 28);

        assert!(matches!(
            layout.data_directory.entries[0].signature,
            ConstantSignature::Scalar(ScalarTag::Integer32)
        ));
        assert!(matches!(
            layout.data_directory.entries[1].signature,
            ConstantSignature::Scalar(ScalarTag::Float32)
        ));
        assert!(matches!(
            layout.data_directory.entries[2].signature,
            ConstantSignature::String
        ));
    }

    #[test]
    fn data_directory_all_type_tags_roundtrip()
    {
        let entries = [
            (1u32, 0u32, ConstantSignature::Scalar(ScalarTag::Integer32)),
            (1u32, 1u32, ConstantSignature::Scalar(ScalarTag::Integer64)),
            (1u32, 2u32, ConstantSignature::Scalar(ScalarTag::Float32)),
            (1u32, 3u32, ConstantSignature::Scalar(ScalarTag::Float64)),
            (1u32, 4u32, ConstantSignature::String),
            (1u32, 5u32, ConstantSignature::ValueType { type_index: 42 }),
        ];
        let payload = vec![0u8; 6];

        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&entries, &payload)
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        let sigs: Vec<ConstantSignature> = layout.data_directory.entries.iter().map(|e| e.signature).collect();

        assert!(matches!(sigs[0], ConstantSignature::Scalar(ScalarTag::Integer32)));
        assert!(matches!(sigs[1], ConstantSignature::Scalar(ScalarTag::Integer64)));
        assert!(matches!(sigs[2], ConstantSignature::Scalar(ScalarTag::Float32)));
        assert!(matches!(sigs[3], ConstantSignature::Scalar(ScalarTag::Float64)));
        assert!(matches!(sigs[4], ConstantSignature::String));
        assert!(matches!(sigs[5], ConstantSignature::ValueType { type_index: 42 }));
    }

    #[test]
    fn data_directory_invalid_type_tag_returns_error()
    {
        // Generate a perfectly valid layout first.
        // We use `ConstantSignature::String` because we know it compiles to exactly
        // one byte (`0x01`), making it easy to swap out.
        let entry = (4u32, 0u32, ConstantSignature::String);
        let mut bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&[entry], &[0u8; 4])
            .build();

        // The data directory payload is appended at the very end of the file.
        // Layout at the end: [..signature byte, 4 bytes (data_length), 4 bytes (data)]
        // This means our magic byte `0x01` is exactly 9 bytes from the end.
        let magic_byte_index = bytes.len() - 9;

        // Sanity check to guarantee we are targeting the right byte
        assert_eq!(bytes[magic_byte_index], 0x01);

        // 0x05 is not a defined ConstantSignature variant. Corrupt the byte!
        bytes[magic_byte_index] = 0x05;

        // The parse must fail rather than silently produce a garbage value.
        assert!(FileLayout::read_le(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn data_directory_entries_byte_size()
    {
        // entries_byte_size = count * size_of::<DataHeader>()
        let entries = [
            (1u32, 0u32, ConstantSignature::Scalar(ScalarTag::Integer32)),
            (1u32, 1u32, ConstantSignature::Scalar(ScalarTag::Integer32)),
        ];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(&entries, &[0u8; 2])
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(layout.data_directory.entries_byte_size(), 2 * size_of::<DataHeader>());
    }

    #[test]
    fn data_directory_raw_bytes_preserved()
    {
        let payload: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &[])
            .data_directory(
                &[(8u32, 0u32, ConstantSignature::Scalar(ScalarTag::Integer32))],
                &payload,
            )
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(layout.data_directory.data, payload);
    }

    // ── Full compound FileLayout ───────────────────────────────────────────────

    #[test]
    fn full_layout_all_sections_populated()
    {
        let links = [(0u16, dummy_symbol_id(0xAA), 50u32)];
        let syms = [
            (dummy_symbol_id(0x11), 0u8, Some(0u32)),
            (dummy_symbol_id(0x22), 1u8, None),
        ];

        let funcs = [(dummy_symbol_id(0x11), 0u32, 4u32, 3u32, 6u32, 2u8, 0b0000_0001u8)];
        let bytecode = vec![0x01, 0x02, 0x03, 0x04];

        let data_entries = [(4u32, 0u32, ConstantSignature::Scalar(ScalarTag::Float64))];
        let raw_data = vec![0xDE, 0xAD, 0xBE, 0xEF];

        let bytes = FileBuilder::new()
            .header(2, 1, dummy_symbol_id(0x99), 0b0000_0001)
            .link_table(&links)
            .symbol_table(&syms)
            .code_directory(&funcs, &bytecode)
            .data_directory(&data_entries, &raw_data)
            .build();

        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(layout.header.file_version, 2);
        assert!(layout.header.flags.contains(FileFlags::HAS_DEBUG));

        assert_eq!(layout.link_table.entries.len(), 1);
        assert_eq!(layout.link_table.entries[0].module_path, 50);

        assert_eq!(layout.symbol_table.symbols.len(), 2);
        assert!(matches!(
            layout.symbol_table.symbols[0].kind,
            SymbolKind::Function { body: 0 }
        ));
        assert!(matches!(layout.symbol_table.symbols[1].kind, SymbolKind::Type { type_index: 0 }));

        assert_eq!(layout.code_directory.function_count(), 1);
        assert!(
            layout.code_directory.functions[0]
                .flags
                .contains(FunctionFlags::ENTRYPOINT)
        );
        assert_eq!(layout.code_directory.functions[0].param_count, 2);
        assert_eq!(layout.code_directory.bytecode, bytecode);

        assert_eq!(layout.data_directory.entries.len(), 1);
        assert!(matches!(
            layout.data_directory.entries[0].signature,
            ConstantSignature::Scalar(ScalarTag::Float64)
        ));
        assert_eq!(layout.data_directory.data, raw_data);
    }

    #[test]
    fn full_layout_large_bytecode()
    {
        let bytecode: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&[], &bytecode)
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.bytecode_size(), 1024);
        assert_eq!(layout.code_directory.bytecode, bytecode);
    }

    #[test]
    fn full_layout_many_functions()
    {
        // (symbol_id, index, length, maxlocals, maxstack, param_count, flags)
        let funcs: Vec<([u8; 16], u32, u32, u32, u32, u8, u8)> = (0..50)
            .map(|i| {
                (
                    dummy_symbol_id(i as u8),
                    i * 10,
                    10,
                    i,
                    i * 2,
                    i as u8,                    // param_count
                    if i == 0 { 1 } else { 0 }, // flags
                )
            })
            .collect();
        let bytes = FileBuilder::new()
            .header(1, 1, dummy_symbol_id(0), 0)
            .link_table(&[])
            .symbol_table(&[])
            .code_directory(&funcs, &[0u8; 500])
            .data_directory(&[], &[])
            .build();
        let layout = FileLayout::read_le(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(layout.code_directory.function_count(), 50);
        assert!(
            layout.code_directory.functions[0]
                .flags
                .contains(FunctionFlags::ENTRYPOINT)
        );
        for (i, f) in layout.code_directory.functions[1..].iter().enumerate()
        {
            assert!(f.flags.is_empty(), "function {i} should not be an entrypoint");
            assert_eq!(f.param_count, (i + 1) as u8);
        }
    }

    #[test]
    fn truncated_input_returns_error()
    {
        // Cut the valid buffer in half.
        let bytes = minimal_layout(0);
        let half = bytes.len() / 2;
        assert!(FileLayout::read_le(&mut Cursor::new(&bytes[..half])).is_err());
    }

    #[test]
    fn empty_input_returns_error()
    {
        assert!(FileLayout::read_le(&mut Cursor::new(vec![])).is_err());
    }
}
