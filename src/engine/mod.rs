pub mod context;
pub mod opcode_handler;
pub mod opcodes;

use crate::{
    engine::{context::ExecutionContext, opcode_handler::ExecutionError},
    loader::{Loader, LoaderError},
    memory::{heap::heap::Heap, stack::Stack},
};

#[derive(Debug, Clone, Copy)]
pub enum RunnerError
{
    CannotAcquireEntrypoint,
    StackOverflow,
    ExecutionError(ExecutionError),
    ProgramCounterOverflow,
    LoaderFailure,
}

impl From<LoaderError> for RunnerError
{
    fn from(_value: LoaderError) -> Self
    {
        Self::LoaderFailure
    }
}

impl From<ExecutionError> for RunnerError
{
    fn from(value: ExecutionError) -> Self
    {
        Self::ExecutionError(value)
    }
}

pub struct Runner<'a, 'b>
{
    stack: &'a mut Stack,
    loader: &'a mut Loader<'b>,
    heap: &'a mut Heap,
}

impl<'a, 'b> Runner<'a, 'b>
where
    'b: 'a,
{
    pub fn new(stack: &'a mut Stack, loader: &'a mut Loader<'b>, heap: &'a mut Heap) -> Self
    {
        Self { stack, loader, heap }
    }

    pub fn run(&mut self) -> Result<(), RunnerError>
    {
        ExecutionContext::run(self.loader, self.stack, self.heap)
    }
}

// TODO: Testing
