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
        let (magic, rem) = split_off!(u64, input)?;
        let (version, rem) = rem.split_first()?;
        let (constant_count, rem) = split_off!(u16, rem)?;
        let (constant_pool, rem) = Table::new(constant_count.into(), rem)?;
        let (function_count, rem) = split_off!(u16, rem)?;
        let (functions, _) = FunctionInfo::get_all_functions(rem, &constant_pool)?;

        Some(Self {
            magic,
            version: *version,
            constant_count,
            constant_pool,
            function_count,
            functions,
        })
    }
}

#[derive(Clone, Copy)]
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
        (8, &|x| {
            Some(TableEntry::Long(u64::from_le_bytes(x.try_into().ok()?)))
        }),
        (4, &|x| {
            Some(TableEntry::Float(f32::from_le_bytes(x.try_into().ok()?)))
        }),
        (8, &|x| {
            Some(TableEntry::Double(f64::from_le_bytes(x.try_into().ok()?)))
        }),
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
                [] => return None,
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
        let &(symbol_operand_count, symbol_handler) =
            Directive::HANDLERS.get(<usize>::from(Directive::SYMBOL))?;
        let (symbol_operands, rem_dirs) = input.split_at_checked(symbol_operand_count)?;

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

                    let name = table.entries.get(name_idx)?;
                    let descriptor = table.entries.get(descriptor_idx)?;

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

        let mut directives: Vec<Directive> = vec![];
        let mut remaining = rem_dirs;
        loop
        {
            match *remaining
            {
                [Directive::OPCODE, Directive::SYMBOL, ..] => return None, // Duplicate symbol directive
                [Directive::OPCODE, x, ref res @ ..] =>
                {
                    let &(operand_count, handler) = Directive::HANDLERS.get(<usize>::from(x))?;
                    let (operands, rem) = res.split_at_checked(operand_count)?;

                    directives.push(handler(operands)?);

                    remaining = rem;
                }
                [..] => break,
            }
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
