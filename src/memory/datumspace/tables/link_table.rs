use crate::{loader::SymbolId, memory::datumspace::datum::InlinedString};


#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Link<'a>
{
    id: SymbolId,
    path: InlinedString<'a>,
}

impl Link<'_>
{

}
