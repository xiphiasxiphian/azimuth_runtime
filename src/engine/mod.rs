pub mod context;
pub mod opcode_handler;
pub mod opcodes;

use crate::{
    engine::{
        context::ExecutionContext,
        opcode_handler::{ExecutionError, InstructionResult, exec_instruction},
    },
    loader::{self, Loader, LoaderError},
    memory::stack::{Stack, entry},
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

pub struct Runner<'a, 'b>
{
    stack: &'a mut Stack,
    loader: &'a mut Loader<'b>,
    // heap
}

impl<'a, 'b> Runner<'a, 'b>
where
    'b: 'a,
{
    pub fn new(stack: &'a mut Stack, loader: &'a mut Loader<'b>) -> Self
    {
        Self { stack, loader }
    }

    pub fn run(&mut self) -> Result<(), RunnerError>
    {
        /*
         * TODO:
         * - Set the contexts (loader and stack) so that jumping between functions works
         * - Set up infrastructure to actually work out where a function is, load it
         * and then execute it
         * - Introduce bytecode instructions for running functions
         * - TESTING
         */

        ExecutionContext::run(self.loader, self.stack)
    }
}
