use crate::loader::SymbolId;

#[derive(Clone, Copy)]
pub enum SymbolKind
{
    Function
    {
        index: u32
    },
    Type
    {
        index: u32,
    }
}

#[derive(Clone, Copy)]
pub struct Symbol
{
    pub kind: SymbolKind,
    pub id: SymbolId,
}
