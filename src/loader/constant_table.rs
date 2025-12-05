// This is a more formalised wrapper around the idea of the constant table.
// In the future this can be more "referency" as things will instead be stored in metaspace

use crate::{
    engine::stack::StackFrame,
    loader::parser::{Table, TableEntry},
};

pub type ConstantTableIndex = u32;

#[derive(Debug)]
pub struct ConstantTable<'a>
{
    entries: Vec<Constant<'a>>,
}

/// A Constant stored within the constant table.
///
/// These are roughly direct copies of the ones stored within
/// the binary itself, but abstracted out for the sake
/// of ease of use.
///
/// ## Variants
/// There are 5 main types of Constant:
///
/// `Unsigned32` - Stores a `u32` (also called `int` in some languages).
/// It is important to note that this exists
/// out of pure convenience, as it will get padded up to 64-bits when loaded
/// onto the stack.
///
/// `Unsigned64` - Stores a `u64` (also called `long` in some languages).
///
/// `Float32` - Stores a `f32`. It is important to note that, unlike `Unsigned32`,
/// this doesn't exist just for convenience, as sometimes representing different
/// floating point precisions can be important. However, like `Unsigned32`, this will
/// still get padded with 0s to 64-bits when loaded onto the stack.
///
/// `Float64` - Stores a `f64` (also called `double` in some languages)
///
/// `String` - Stores a string reference (the string data is stored in metaspace)
#[derive(Debug, Clone, Copy)]
pub enum Constant<'a>
{
    Unsigned32(u32),
    Unsigned64(u64),
    Float32(f32),
    Float64(f64),
    String(&'a str),
}

impl<'a> Constant<'a>
{
    pub fn from_parsed_entry(entry: &'a TableEntry) -> Self
    {
        match *entry
        {
            TableEntry::Integer(x) => Self::Unsigned32(x),
            TableEntry::Long(x) => Self::Unsigned64(x),
            TableEntry::Float(x) => Self::Float32(x),
            TableEntry::Double(x) => Self::Float64(x),
            TableEntry::String(ref string) => Self::String(string.as_str()),
        }
    }
}

impl<'a> ConstantTable<'a>
{
    pub fn from_parsed_table(table: &'a Table) -> Self
    {
        Self {
            entries: table.entries().iter().map(Constant::from_parsed_entry).collect(),
        }
    }

    pub fn get_entry(&self, index: ConstantTableIndex) -> Option<&Constant<'a>>
    {
        self.entries.get(index as usize)
    }

    /// Pushes a constant onto the stack, converting each constant type into a stack
    /// value depending on its type.
    pub fn push_entry(&self, stack: &mut StackFrame, index: ConstantTableIndex) -> Option<bool>
    {
        self.get_entry(index)
            .map(|x| match *x
            {
                Constant::Unsigned32(x) => stack.push(x.into()), // expanded into u64
                Constant::Unsigned64(x) => stack.push(x),
                Constant::Float32(x) => stack.push(x.to_bits().into()), // expanded and tranmuted into u64
                Constant::Float64(x) => stack.push(x.to_bits()), // transmuted into u64
                // Strings a represented on the stack with their reference
                Constant::String(string) => stack.push(string.as_ptr() as u64),

            })
    }
}
