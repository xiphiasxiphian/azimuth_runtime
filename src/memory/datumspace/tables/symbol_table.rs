use crate::{loader::SymbolId, memory::datumspace::datum::{InlinedString, Offset}};

pub enum SymbolKind
{
    Function {
        index: u32,
    },
}

pub struct Symbol<'a>
{
    kind: SymbolKind,
    qualified_name: InlinedString<'a>,
    id: SymbolId
}
