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

    pub fn push_entry(&self, stack: &mut StackFrame, index: ConstantTableIndex) -> Option<bool>
    {
        self.get_entry(index)
            .map(|x| match *x
            {
                Constant::Unsigned32(x) => stack.push(x.into()),
                Constant::Unsigned64(x) => stack.push(x),
                Constant::Float32(x) => stack.push(x.to_bits().into()),
                Constant::Float64(x) => stack.push(x.to_bits()),
                Constant::String(string) => stack.push(string.as_ptr() as u64),
            })
    }
}
