use crate::{
    loader::SymbolId,
    memory::datumspace::datum::{InlinedString, Offset},
};

#[derive(Clone, Copy)]
pub enum SymbolKind
{
    Function
    {
        index: u32
    },
}

#[derive(Clone, Copy)]
pub struct Symbol
{
    pub kind: SymbolKind,
    pub id: SymbolId,
}
