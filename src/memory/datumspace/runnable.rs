use crate::{
    memory::datumspace::{DatumAllocator, DatumspaceError, datum::BlockLocation},
};

#[derive(Debug, Clone, Copy)]
pub enum Runnable
{
    Unresolved(UnresolvedRunnable),
    Function(Function),
}

impl Runnable
{

}

#[derive(Clone, Copy, Debug)]
pub struct UnresolvedRunnable
{
    pub loc: BlockLocation,

}

#[derive(Clone, Copy, Debug)]
pub struct Function
{
    pub maxstack: usize,
    pub maxlocals: usize,
    pub bytecode: BlockLocation,
}
