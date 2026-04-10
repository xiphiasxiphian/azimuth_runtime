use bitflags::bitflags;

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

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FunctionFlags: u8 {
        const ENTRYPOINT = 0b0000_0001;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Function
{
    pub maxstack: u32,
    pub maxlocals: u32,
    pub bytecode: BlockLocation,
    pub flags: FunctionFlags,
}
