use crate::engine::opcodes::Opcode;

const MAGIC_STRING: &[u8; 8] = b"azimuth\0";
pub const MAGIC_NUMBER: u64 = u64::from_le_bytes(*MAGIC_STRING);

macro_rules! split_off {
    ($t:ty, $input:ident) => {
        $input
            .split_at_checked(size_of::<$t>())
            .and_then(|(x, y)| Some((<$t>::from_le_bytes(x.try_into().ok()?), y)))
    };
}

type DirectiveHandler = &'static dyn Fn(&[u8]) -> Option<Directive>;
type TableTypeHandler = &'static dyn Fn(&[u8]) -> Option<TableEntry>;

struct FileParser<'a>
{
    remaining: &'a [u8]
}

impl<'a> FileParser<'a>
{
    pub fn new(input: &'a [u8]) -> Self
    {
        Self { remaining: input }
    }

    pub fn parse_off<T, F>(&mut self, parser: F) -> Option<T>
    where
        F: Fn(&'a [u8]) -> Option<(T, &'a [u8])>,
    {
        let (value, rem) = parser(self.remaining)?;
        self.remaining = rem;
        Some(value)
    }
}

struct FileLayout
{
    magic: u64,
    version: u8,
    constant_count: u16,
    constant_pool: Table,
    function_count: u16,
    functions: Vec<FunctionInfo>,
}

impl FileLayout
{
    pub fn from_bytes(input: &[u8]) -> Option<Self>
    {
        let mut parser = FileParser::new(input);

        let magic = parser.parse_off(|x| split_off!(u64, x))?;
        let &version = parser.parse_off(|x| x.split_first())?;
        let constant_count = parser.parse_off(|x| split_off!(u16, x))?;
        let constant_pool = parser.parse_off(|x| Table::new(constant_count.into(), x))?;
        let function_count = parser.parse_off(|x| split_off!(u16, x))?;
        let functions = parser.parse_off(|x| FunctionInfo::get_all_functions(x, &constant_pool))?;

        Some(Self {
            magic,
            version,
            constant_count,
            constant_pool,
            function_count,
            functions,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum TableEntry
{
    Integer(u32),
    Long(u64),
    Float(f32),
    Double(f64),
}

impl TableEntry
{
    pub const HANDLERS: [(usize, TableTypeHandler); 4] = [
        (4, &|x| {
            Some(TableEntry::Integer(u32::from_le_bytes(x.try_into().ok()?)))
        }),
        (8, &|x| Some(TableEntry::Long(u64::from_le_bytes(x.try_into().ok()?)))),
        (4, &|x| Some(TableEntry::Float(f32::from_le_bytes(x.try_into().ok()?)))),
        (8, &|x| Some(TableEntry::Double(f64::from_le_bytes(x.try_into().ok()?)))),
    ];
}

struct Table
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
        {
            match *remaining
            {
                [] => return None, // There were not enough entries, therefore the file is malformed
                [tag, ref res @ ..] =>
                {
                    let &(operand, handler) = TableEntry::HANDLERS.get(<usize>::from(tag))?;

                    let (operands, rem) = res.split_at_checked(operand)?;
                    entries.push(handler(operands)?);

                    remaining = rem;
                }
            }
        }

        Some((Self { entries }, remaining))
    }

    pub fn get(&self, idx: usize) -> Option<&TableEntry>
    {
        self.entries.get(idx)
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Directive
{
    Symbol(u16, u16), // (name_index, descriptor_index)
    Start,
    MaxStack(u16),
    MaxLocals(u16),
}

impl Directive
{
    const OPCODE: u8 = Opcode::Directive as u8;
    const SYMBOL: u8 = 0;

    const HEADER_SIZE: usize = 2; // Opcode (1 byte) + Directive Type (1 byte)

    const HANDLERS: [(usize, DirectiveHandler); 4] = [
        (4, &|x| {
            Some(Directive::Symbol(
                u16::from_le_bytes(x[0..2].try_into().ok()?),
                u16::from_le_bytes(x[2..4].try_into().ok()?),
            ))
        }),
        (0, &|_| Some(Directive::Start)),
        (2, &|x| {
            Some(Directive::MaxStack(u16::from_le_bytes(x.try_into().ok()?)))
        }),
        (2, &|x| {
            Some(Directive::MaxLocals(u16::from_le_bytes(x.try_into().ok()?)))
        }),
    ];
}

struct FunctionInfo
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
    pub fn new<'a>(input: &'a [u8], table: &Table) -> Option<(Self, &'a [u8])>
    {
        // Get symbol directive. The symbol directive
        // should be Directive 0, so get its entry in the handler array
        let &(symbol_operand_count, symbol_handler) = Directive::HANDLERS.get(<usize>::from(Directive::SYMBOL))?;
        let (symbol_directive, rem_dirs) = input.split_at_checked(symbol_operand_count + Directive::HEADER_SIZE)?;
        let symbol_operands = symbol_directive.get(Directive::HEADER_SIZE..(symbol_operand_count + Directive::HEADER_SIZE))?;

        let (_, descriptor): (_, u32) = symbol_handler(symbol_operands).and_then(|x| {
            match x
            {
                Directive::Symbol(name_index, descriptor_index) =>
                {
                    // Even thought the name is not needed here, it is
                    // important still to verify that it is a valid constant pool entry,
                    // and does in fact refer to a string entry

                    let name_idx = <usize>::from(name_index);
                    let descriptor_idx = <usize>::from(descriptor_index);

                    // Get the name and descriptor from the constant pool.
                    // This will also check whether the given indices are in fact valid.
                    let name = table.get(name_idx)?;
                    let descriptor = table.get(descriptor_idx)?;

                    match (name, descriptor)
                    {
                        // The name should refer to a String, and the descriptor should refer to an Integer
                        (&_, &TableEntry::Integer(x)) => Some((name, x)),
                        _ => None,
                    }
                }
                _ => None, // Something has gone really wrong if this triggers
            }
        })?;

        println!("Parsed symbol directive as {descriptor}");

        let mut directives: Vec<Directive> = vec![];
        let mut remaining = rem_dirs;

        // Loop through the bytes until it doesn't represent a directive anymore
        while let &[Directive::OPCODE, x, ref res @ ..] = remaining
        {
            if x == Directive::SYMBOL { return None; }
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
            0, 10, 0, 0, 0,  // Integer 10
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
        let data: [u8; 10] = [
            Directive::OPCODE,
            Directive::SYMBOL,
            0, 0, // name index
            1, 0, // descriptor index
            // Code (4 bytes)
            0x01, 0x02, 0x03, 0x04,
        ];
        let table = Table {
            entries: vec![
                TableEntry::Integer(0), // name index
                TableEntry::Integer(4), // descriptor index
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
{

}
