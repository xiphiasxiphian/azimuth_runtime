use crate::{engine::opcodes::Opcode, guard, loader::runnable::Runnable};

const MAGIC_STRING: &[u8; 8] = b"azimuth\0";
pub const MAGIC_NUMBER: u64 = u64::from_le_bytes(*MAGIC_STRING);

// Convert a set of bytes into a numeric type
macro_rules! bytes_to_numeric {
    ($t:ty, $input:expr) => {
        <$t>::from_le_bytes(*$input.first_chunk()?)
    };
}

// Macro to speed up splitting of a specific bit of the data into a specific
// numeric type
macro_rules! split_off {
    ($t:ty, $input:ident) => {
        $input
            .split_at_checked(size_of::<$t>())
            .and_then(|(x, y)| Some((bytes_to_numeric!($t, x), y)))
    };
}

type DirectiveHandler = &'static dyn Fn(&[u8]) -> Option<Directive>; // Creates a handler
type TableTypeHandler = &'static dyn Fn(&[u8]) -> Option<(TableEntry, usize)>; // Creates a table

struct FileParser<'a>
{
    remaining: &'a [u8],
}

impl<'a> FileParser<'a>
{
    pub fn new(input: &'a [u8]) -> Self
    {
        Self { remaining: input }
    }

    /// Create a type based on a given parser
    pub fn parse_off<T, F>(&mut self, parser: F) -> Option<T>
    where
        F: Fn(&'a [u8]) -> Option<(T, &'a [u8])>,
    {
        let (value, rem) = parser(self.remaining)?;
        self.remaining = rem;
        Some(value)
    }
}

pub struct FileLayout
{
    magic: u64,
    version: u8,
    constant_count: u32,
    constant_pool: Table,
    functions: Vec<FunctionInfo>,
}

impl FileLayout
{
    /// Parse the direct information from a raw file, representing its format as closely as possible.
    pub fn from_bytes(input: &[u8]) -> Option<Self>
    {
        let mut parser = FileParser::new(input);

        let magic = parser.parse_off(|x| split_off!(u64, x))?; // Magic Number
        let &version = parser.parse_off(|x| x.split_first())?; // Version Number
        let constant_count = parser.parse_off(|x| split_off!(u32, x))?; // Number of constants
        let constant_pool = parser.parse_off(|x| Table::new(constant_count as usize, x))?; // Constant Table
        let functions = parser.parse_off(|x| FunctionInfo::get_all_functions(x, &constant_pool))?; // Functions

        Some(Self {
            magic,
            version,
            constant_count,
            constant_pool,
            functions,
        })
    }

    pub fn functions(&self) -> &[FunctionInfo]
    {
        self.functions.as_slice()
    }

    pub fn constants(&self) -> &Table
    {
        &self.constant_pool
    }
}

#[derive(Debug, Clone)]
pub enum TableEntry
{
    Integer(u32),
    Long(u64),
    Float(f32),
    Double(f64),
    String(String), // This can eventually be a reference to a metaspace string
}

impl TableEntry
{
    pub const HANDLERS: [TableTypeHandler; 5] = [
        &|x| Some((TableEntry::Integer(bytes_to_numeric!(u32, x)), 4)),
        &|x| Some((TableEntry::Long(bytes_to_numeric!(u64, x)), 8)),
        &|x| Some((TableEntry::Float(f32::from_bits(bytes_to_numeric!(u32, x))), 4)),
        &|x| Some((TableEntry::Double(f64::from_bits(bytes_to_numeric!(u64, x))), 8)),
        &|x| {
            let str_len = bytes_to_numeric!(u32, x) as usize;
            let str_bytes = x.get(size_of::<u32>()..(size_of::<u32>() + str_len))?;
            let string = String::from_utf8(str_bytes.to_vec()).ok()?;
            Some((TableEntry::String(string), size_of::<u32>() + str_len))
        },
    ];
}

#[derive(Debug)]
pub struct Table
{
    entries: Vec<TableEntry>,
}

impl Table
{
    pub fn new(count: usize, from: &[u8]) -> Option<(Self, &[u8])>
    {
        let mut entries: Vec<TableEntry> = Vec::with_capacity(count);

        let mut remaining: &[u8] = from;
        for _ in 0..count
        // Parse entries based on the count previously given
        {
            match *remaining
            {
                [] => return None, // There were not enough entries, therefore the file is malformed
                [tag, ref res @ ..] =>
                // Parse the entry
                {
                    let (result, operands) = TableEntry::HANDLERS.get(<usize>::from(tag))?(res)?;

                    let (_, rem) = res.split_at_checked(operands)?;
                    entries.push(result);

                    remaining = rem;
                }
            }
        }

        Some((Self { entries }, remaining))
    }

    pub fn get(&self, idx: u32) -> Option<&TableEntry>
    {
        self.entries.get(idx as usize)
    }

    pub fn entries(&self) -> &[TableEntry]
    {
        &self.entries
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Directive
{
    Symbol(u32, u32), // (name_index, descriptor_index)
    Start,
    MaxStack(u16),  // max_stack
    MaxLocals(u16), // max_locals
}

impl Directive
{
    const OPCODE: u8 = Opcode::Directive as u8; // Opcode for a directive
    const SYMBOL: u8 = 0; // The symbol directive is important and should always be 0

    const HEADER_SIZE: usize = 2; // Opcode (1 byte) + Directive Type (1 byte)

    const HANDLERS: [(usize, DirectiveHandler); 4] = [
        (8, &|x| {
            Some(Directive::Symbol(
                u32::from_le_bytes(x[0..4].try_into().ok()?),
                u32::from_le_bytes(x[4..8].try_into().ok()?),
            ))
        }),
        (0, &|_| Some(Directive::Start)),
        (2, &|x| Some(Directive::MaxStack(bytes_to_numeric!(u16, x)))),
        (2, &|x| Some(Directive::MaxLocals(bytes_to_numeric!(u16, x)))),
    ];
}

#[derive(Debug)]
pub struct FunctionInfo
{
    directives: Vec<Directive>,

    // In the future this code section will be able to be a byte slice
    // (&[u8]) rather than an owned vector as the actual data will be stored in
    // metaspace somewhere.
    // However, as metaspace doesnt exist yet, right now it has to be
    // owned.
    code: Vec<u8>,
}

impl FunctionInfo
{
    pub fn new<'b>(input: &'b [u8], table: &Table) -> Option<(Self, &'b [u8])>
    {
        // Get symbol directive. The symbol directive
        // should be Directive 0, so get its entry in the handler array
        let &(symbol_operand_byte_count, symbol_handler) = Directive::HANDLERS.get(<usize>::from(Directive::SYMBOL))?;
        let (symbol_directive, rem_dirs) =
            input.split_at_checked(symbol_operand_byte_count + Directive::HEADER_SIZE)?;

        let symbol_operands = symbol_directive.get(Directive::HEADER_SIZE..)?;

        let (name, descriptor): (&str, u32) = symbol_handler(symbol_operands).and_then(|x| {
            match x
            {
                Directive::Symbol(name_index, code_count) =>
                {
                    // Even thought the name is not needed here, it is
                    // important still to verify that it is a valid constant pool entry,
                    // and does in fact refer to a string entry

                    // Get the name and descriptor from the constant pool.
                    // This will also check whether the given indices are in fact valid.
                    let name = table.get(name_index)?;

                    match *name
                    {
                        // The name should refer to a String, and the descriptor should refer to an Integer
                        TableEntry::String(ref name_str) => Some((name_str.as_str(), code_count)),
                        _ => None,
                    }
                }
                _ => None, // Something has gone really wrong if this triggers
            }
        })?;

        let mut directives: Vec<Directive> = vec![];
        let mut remaining = rem_dirs;

        // Loop through the bytes until it doesn't represent a directive anymore
        while let &[Directive::OPCODE, x, ref res @ ..] = remaining
        {
            // This means that there has been a second symbol directive which isnt
            // legal
            guard!(x != Directive::SYMBOL);

            // Parse the found directive
            let &(operand_count, handler) = Directive::HANDLERS.get(<usize>::from(x))?;
            let (operands, rem) = res.split_at_checked(operand_count)?;

            directives.push(handler(operands)?);

            remaining = rem;
        }

        #[expect(
            clippy::expect_used,
            reason = "Running this program on a less than 32-bit architecture isn't supported"
        )]
        let (code_slice, remaining) = remaining.split_at_checked(
            descriptor
                .try_into()
                .expect("Running on a none 32-bit or 64-bit architecture. How? Why?"),
        )?;

        Some((
            Self {
                directives,
                code: code_slice.to_vec(),
            },
            remaining,
        ))
    }

    pub fn get_all_functions<'a>(input: &'a [u8], table: &Table) -> Option<(Vec<Self>, &'a [u8])>
    {
        let mut functions = vec![];
        let mut remaining = input;
        while let &[Directive::OPCODE, Directive::SYMBOL, ..] = remaining
        // There is another function to read
        {
            let (function, rem) = Self::new(remaining, table)?;
            functions.push(function);
            remaining = rem;
        }

        Some((functions, remaining))
    }

    /// Turn a raw parsed `FunctionInfo` into a usable `Runnable`, with safety checks
    pub fn into_runnable(&self) -> Option<Runnable<'_>>
    {
        Runnable::from_parsed_data(&self.directives, &self.code)
    }

    pub fn has_directive(&self, directive: Directive) -> bool
    {
        self.directives.contains(&directive)
    }
}

#[cfg(test)]
mod table_tests
{
    use super::*;

    #[test]
    fn empty_table()
    {
        let data: [u8; 0] = [];
        let (table, rem) = Table::new(0, &data).expect("Failed to parse empty table");
        assert!(table.entries.is_empty());
        assert!(rem.is_empty());
    }

    #[test]
    fn homogeneous_table()
    {
        let data: [u8; 15] = [
            0, 10, 0, 0, 0, // Integer 10
            0, 20, 0, 0, 0, // Integer 20
            0, 30, 0, 0, 0, // Integer 30
        ];
        let (table, rem) = Table::new(3, &data).expect("Failed to parse homogeneous table");
        assert_eq!(table.entries.len(), 3);
        assert!(matches!(table.get(0), Some(TableEntry::Integer(10))));
        assert!(matches!(table.get(1), Some(TableEntry::Integer(20))));
        assert!(matches!(table.get(2), Some(TableEntry::Integer(30))));
        assert!(rem.is_empty());
    }

    #[test]
    fn heterogeneous_table()
    {
        let data: [u8; 28] = [
            0, 10, 0, 0, 0, // Integer 10
            1, 100, 0, 0, 0, 0, 0, 0, 0, // Long 100
            2, 0, 0, 128, 63, // Float 1.0
            3, 0, 0, 0, 0, 0, 0, 240, 63, // Double 1.0
        ];
        let (table, rem) = Table::new(4, &data).expect("Failed to parse heterogeneous table");
        assert_eq!(table.entries.len(), 4);
        assert!(matches!(table.get(0), Some(TableEntry::Integer(10))));
        assert!(matches!(table.get(1), Some(TableEntry::Long(100))));
        assert!(matches!(table.get(2), Some(TableEntry::Float(f)) if (f - 1.0).abs() < f32::EPSILON));
        assert!(matches!(table.get(3), Some(TableEntry::Double(d)) if (d - 1.0).abs() < f64::EPSILON));
        assert!(rem.is_empty());
    }
}

#[cfg(test)]
mod function_info_tests
{
    use super::*;

    #[test]
    fn basic_function()
    {
        // Function with symbol directive and no other directives
        let data: [u8; 14] = [
            Directive::OPCODE,
            Directive::SYMBOL,
            0,
            0,
            0,
            0, // name index
            4,
            0,
            0,
            0, // code count
            // Code (4 bytes)
            0x01,
            0x02,
            0x03,
            0x04,
        ];
        let table = Table {
            entries: vec![
                TableEntry::String("main".into()), // name index
                TableEntry::Integer(4),            // descriptor index
            ],
        };

        let (function, rem) = FunctionInfo::new(&data, &table).expect("Failed to parse simple function");
        assert_eq!(function.directives.len(), 0); // Doesn't include symbol directive
        assert_eq!(function.code, vec![0x01, 0x02, 0x03, 0x04]);
        assert!(rem.is_empty());
    }
}

#[cfg(test)]
mod parser_tests
{}
