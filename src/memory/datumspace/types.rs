// User defined types (structs etc.)

use num_traits::FromBytes;

struct ArrayParserIter<'a, F>
{
    remaining: &'a [u8],
    parser: F,
}

impl<'a, T, F: Fn(&'a [u8]) -> Option<(T, &'a [u8])>> Iterator for ArrayParserIter<'a, F>
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item>
    {
        let (value, rem) = (self.parser)(self.remaining)?;
        self.remaining = rem;

        Some(value)
    }
}

fn split_array_data<L, const N: usize>(bytes: &[u8]) -> Option<(&[u8], &[u8])>
where
    L: FromBytes<Bytes = [u8; N]> + Into<usize>,
{
    let (len_bytes, rem) = bytes.split_first_chunk()?;
    let length = <L>::from_le_bytes(len_bytes);

    let (array_bytes, rem) = rem.split_at_checked(length.into())?;
    Some((array_bytes, rem))
}

fn parse_variable_array<L, T, F, const N: usize>(bytes: &[u8], transform: F) -> Option<(impl Iterator<Item = T>, &[u8])>
where
    L: FromBytes<Bytes = [u8; N]> + Into<usize>,
    F: Fn(&[u8]) -> Option<(T, &[u8])>,
{
    let (array_bytes, rem) = split_array_data::<u16, _>(bytes)?;
    Some((
        ArrayParserIter {
            remaining: array_bytes,
            parser: transform,
        },
        rem,
    ))
}

fn parse_string<'a, L, const N: usize>(bytes: &'a [u8]) -> Option<(&'a str, &'a [u8])>
where
    L: FromBytes<Bytes = [u8; N]> + Into<usize>,
{
    let (array_bytes, rem) = split_array_data::<u16, _>(bytes)?;
    Some((str::from_utf8(array_bytes).ok()?, rem))
}

#[derive(Clone, Copy)]
pub struct TypeInfo<'a>
{
    id: &'a str,
    raw_field_data: &'a [FieldInfo<'a>],
}

#[derive(Clone, Copy)]
pub struct FieldInfo<'a>
{
    name: &'a str,
    ty: &'a str,
}

impl<'a> TypeInfo<'a>
{
    pub fn from_bytes(bytes: &'a [u8]) -> Option<(Self, &'a [u8])>
    {
        // Parse off the id string
        let (id, rem) = parse_string::<u16, _>(bytes)?;

        // Parse off fields
        let (raw_field_data, rem) = split_array_data::<u16, _>(rem)?;

        Some((
            Self {
                id,
                raw_field_data: &[],
            },
            rem,
        ))
    }

    pub fn id(&self) -> &str
    {
        self.id
    }
}

impl<'a> FieldInfo<'a>
{
    pub fn from_bytes(bytes: &'a [u8]) -> Option<(Self, &'a [u8])>
    {
        let (name, rem) = parse_string::<u16, _>(bytes)?;
        let (ty, rem) = parse_string::<u16, _>(rem)?;

        Some((Self { name, ty }, rem))
    }
}
