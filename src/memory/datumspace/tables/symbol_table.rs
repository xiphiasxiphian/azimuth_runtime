use crate::{loader::SymbolId, memory::datumspace::datum::{InlinedString, Offset}};

#[derive(Clone, Copy)]
pub enum SymbolKind
{
    Function {
        index: u32,
    },
}

#[derive(Clone, Copy)]
pub struct Symbol<'a>
{
    kind: SymbolKind,
    qualified_name: InlinedString<'a>,
    id: SymbolId
}
