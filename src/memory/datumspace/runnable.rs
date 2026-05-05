use bitflags::bitflags;

use crate::memory::datumspace::{DatumAllocator, DatumspaceError, datum::BlockLocation};

#[derive(Debug, Clone, Copy)]
pub enum Runnable
{
    Function(Function),
}

impl Runnable {}

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

impl Function
{
    pub fn setup_info(&self) -> (usize, usize)
    {
        <usize>::try_from(self.maxstack)
            .and_then(|x| Ok((x, <usize>::try_from(self.maxlocals)?)))
            .expect("Running on sub 32-bit architecture")
    }
}
