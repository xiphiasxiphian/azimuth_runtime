const MAGIC_STRING: &str = "azimuth\0";
const MAGIC_NUMBER: u64 = u64::from_le_bytes(*MAGIC_STRING.as_bytes().first_chunk().unwrap());

macro_rules! split_off {
    ($t:ty, $input:ident) => {
        $input
            .split_at_checked(size_of::<$t>())
            .and_then(|(x, y)| Some((<$t>::from_le_bytes(x.try_into().ok()?), y)))
    };
}

struct FileLayout
{
    magic: u64,
    version: u8,
    constant_count: u16,
    constant_pool: Table,
}

impl FileLayout
{
    pub fn from_bytes(input: &[u8]) -> Option<Self>
    {
        let (magic, rem) = split_off!(u64, input)?;
        let (version, rem) = rem.split_first()?;
        let (constant_count, rem) = split_off!(u16, rem)?;
        let (constant_pool, rem) = Table::new(constant_count as usize, rem)?;

        Some(Self {
            magic,
            version: *version,
            constant_count,
            constant_pool
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
    pub const HANDLERS: [(usize, &'static dyn Fn(&[u8]) -> Option<TableEntry>); 4] = [
        (4, &|x| Some(TableEntry::Integer(u32::from_le_bytes(x.try_into().ok()?)))),
        (8, &|x| Some(TableEntry::Long(u64::from_le_bytes(x.try_into().ok()?)))),
        (4, &|x| Some(TableEntry::Float(f32::from_le_bytes(x.try_into().ok()?)))),
        (8, &|x| Some(TableEntry::Double(f64::from_le_bytes(x.try_into().ok()?))))
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
        let mut entries: Vec<TableEntry> =  Vec::with_capacity(count);

        let mut remaining: &[u8] = from;
        for _ in 0..count
        {
            match remaining
            {
                [] => return None,
                [tag, a @ ..] => {
                    let (operand, handler) = TableEntry::HANDLERS[*tag as usize];

                    let (operands, rem) = a.split_at_checked(operand)?;
                    entries.push(handler(operands)?);

                    remaining = rem;
                }
            }
        }

        Some((
            Self {
                entries
            },
            remaining,
        ))
    }
}
