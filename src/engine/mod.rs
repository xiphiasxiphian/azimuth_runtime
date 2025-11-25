pub mod opcode_handler;
pub mod opcodes;
pub mod stack;

use crate::{
    engine::stack::{Stack, StackFrame},
    loader::{Loader, runnable::Runnable},
};

pub enum RunnerError
{
    MissingEntryPoint,
    StackOverflow,
}

pub struct Runner<'a>
{
    stack: &'a mut Stack,
    loader: &'a Loader,
    // heap
}

impl<'a> Runner<'a>
{
    pub fn new(stack: &'a mut Stack, loader: &'a Loader) -> Self
    {
        Self { stack, loader }
    }

    pub fn run(&mut self) -> Result<(), RunnerError>
    {
        let entry_point = self.loader.get_entry_point().ok_or(RunnerError::MissingEntryPoint)?;
        let (maxstack, maxlocals) = entry_point.setup_info();

        let initial_frame = self
            .stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;
        Self::execute(&entry_point, &initial_frame)?;

        Ok(())
    }

    fn execute(runnable: &Runnable, frame: &StackFrame) -> Result<(), RunnerError>
    {
        todo!()
    }
}
