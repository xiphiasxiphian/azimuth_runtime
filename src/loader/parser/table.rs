use crate::loader::parser::bytes_to_numeric;

type TableTypeHandler = &'static dyn Fn(&[u8]) -> Option<(TableEntry, usize)>; // Creates a table

#[derive(Debug, Clone)]
pub enum TableEntry<'a>
{
    Integer(u32),
    Long(u64),
    Float(f32),
    Double(f64),
    String(&'a str),
}

impl TableEntry<'_>
{
    pub const HANDLERS: [TableTypeHandler; 5] = [
        &|x| Some((TableEntry::Integer(bytes_to_numeric!(u32, x)), 4)),
        &|x| Some((TableEntry::Long(bytes_to_numeric!(u64, x)), 8)),
        &|x| Some((TableEntry::Float(f32::from_bits(bytes_to_numeric!(u32, x))), 4)),
        &|x| Some((TableEntry::Double(f64::from_bits(bytes_to_numeric!(u64, x))), 8)),
        &|x| {
            let str_len = bytes_to_numeric!(u32, x) as usize;
            let str_bytes = x.get(size_of::<u32>()..(size_of::<u32>() + str_len))?;
            let string = str::from_utf8(str_bytes).ok()?;
            Some((TableEntry::String(string), size_of::<u32>() + str_len))
        },
    ];
}

#[derive(Debug)]
pub struct Table<'a>
{
    entries: Vec<TableEntry<'a>>,
}

impl<'a> Table<'a>
{
    pub fn new(entries: Vec<TableEntry<'a>>) -> Self
    {
        Self { entries }
    }

    pub fn from_bytes(count: usize, from: &'a [u8]) -> Option<(Self, &'a [u8])>
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

    pub fn byte_size(&self) -> usize
    {
        self.entries
            .iter()
            .fold(0, |acc, x| {
                acc + match x {
                    TableEntry::Long(x) => size_of_val(x),
                    TableEntry::Integer(x) => size_of_val(x),
                    TableEntry::Double(x) => size_of_val(x),
                    TableEntry::Float(x) => size_of_val(x),
                    &TableEntry::String(x) => x.len()
                }
            })
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
        let (table, rem) = Table::from_bytes(0, &data).expect("Failed to parse empty table");
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
        let (table, rem) = Table::from_bytes(3, &data).expect("Failed to parse homogeneous table");
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
        let (table, rem) = Table::from_bytes(4, &data).expect("Failed to parse heterogeneous table");
        assert_eq!(table.entries.len(), 4);
        assert!(matches!(table.get(0), Some(TableEntry::Integer(10))));
        assert!(matches!(table.get(1), Some(TableEntry::Long(100))));
        assert!(matches!(table.get(2), Some(TableEntry::Float(f)) if (f - 1.0).abs() < f32::EPSILON));
        assert!(matches!(table.get(3), Some(TableEntry::Double(d)) if (d - 1.0).abs() < f64::EPSILON));
        assert!(rem.is_empty());
    }
}
