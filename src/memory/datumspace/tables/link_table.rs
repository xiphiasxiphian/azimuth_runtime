use crate::{loader::SymbolId, memory::datumspace::datum::InlinedString};


#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Link<'a>
{
    pub id: SymbolId,
    pub path: InlinedString<'a>,
}

impl Link<'_>
{

}
