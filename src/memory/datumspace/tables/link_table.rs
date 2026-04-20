use crate::{loader::SymbolId, memory::datumspace::datum::InlinedString};


#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Link
{
    pub id: SymbolId,
    pub path: InlinedString,
}
