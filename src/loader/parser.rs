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

        Some(Self {
            magic,
            version: *version,
            constant_count,
            constant_pool,
            function_count,
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
                    let &(operand, handler) = TableEntry::HANDLERS.get(usize::from(tag))?;

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
    Start,
    MaxStack(u16),
    MaxLocals(u16),
}

impl Directive
{


    const OPCODE: u8 = Opcode::Directive as u8;

    const HANDLERS: [(usize, DirectiveHandler); 3] = [
        (0, &|_| Some(Directive::Start)),
        (1, &|x| {
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
    pub fn new(input: &[u8]) -> Option<(Self, &[u8])>
    {
        let mut directives: Vec<Directive> = vec![];
        let mut remaining = input;
        loop
        {
            match *remaining
            {
                [Directive::OPCODE, x, ref res @ ..] =>
                {
                    let &(operand_count, handler) = Directive::HANDLERS.get(usize::from(x))?;
                    let (operands, rem) = res.split_at_checked(operand_count)?;

                    directives.push(handler(operands)?);

                    remaining = rem;
                }
                [_, ..] => break,
                [] => return None,
            }
        }

        Some((
            Self {
                directives,
                code: todo!(),
            },
            remaining,
        ))
    }

    pub fn find_functions_to_end(input: &[u8]) -> Option<Vec<FunctionInfo>>
    {
        todo!()
    }
}
