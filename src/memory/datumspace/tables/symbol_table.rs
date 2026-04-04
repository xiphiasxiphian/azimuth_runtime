use crate::{loader::SymbolId, memory::datumspace::datum::{InlinedString, Offset}};

pub enum SymbolKind
{
    Function {
        offset: Offset,
    },
}

pub struct Symbol<'a>
{
    kind: SymbolKind,
    qualified_name: InlinedString<'a>,
    id: SymbolId
}
