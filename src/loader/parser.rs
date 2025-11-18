const MAGIC_NUMBER: u32 = 0xABBA5EDA;

struct FileLayout
{
    magic: u32,
    version: u8,
    const_pool_count: u16,
    constant_pool: Table,

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
