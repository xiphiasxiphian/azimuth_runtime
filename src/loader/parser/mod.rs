pub mod function;
pub mod table;

use crate::loader::parser::{function::FunctionInfo, table::Table};

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

pub(self) use bytes_to_numeric;

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

pub struct FileLayout<'a>
{
    magic: u64,
    version: u8,
    constant_count: u32,
    constant_pool: Table<'a>,
    functions: Vec<FunctionInfo<'a>>,
}

impl<'a> FileLayout<'a>
{
    /// Parse the direct information from a raw file, representing its format as closely as possible.
    pub fn from_bytes(input: &'a [u8]) -> Option<Self>
    {
        let mut parser = FileParser::new(input);

        let magic = parser.parse_off(|x| split_off!(u64, x))?; // Magic Number
        let &version = parser.parse_off(|x| x.split_first())?; // Version Number
        let constant_count = parser.parse_off(|x| split_off!(u32, x))?; // Number of constants
        let constant_pool = parser.parse_off(|x| Table::from_bytes(constant_count as usize, x))?; // Constant Table
        let functions = parser.parse_off(|x| FunctionInfo::get_all_functions(x))?; // Functions

        Some(Self {
            magic,
            version,
            constant_count,
            constant_pool,
            functions,
        })
    }
}

#[cfg(test)]
mod parser_tests
{}
