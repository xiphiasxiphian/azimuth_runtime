pub mod opcode_handler;
pub mod opcodes;

use crate::{
    engine::opcode_handler::{ExecutionError, InstructionResult, exec_instruction},
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
    fn from(_value: LoaderError) -> Self {
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
    'b: 'a
{
    pub fn new(stack: &'a mut Stack, loader: &'a mut Loader<'b>) -> Self
    {
        Self { stack, loader }
    }

    pub fn run(&mut self) -> Result<(), RunnerError>
    {
        // Get the initial loader context
        let mut loader_context = self.loader.initial_context()?;

        // TEMP: while moving between functions isn't defined yet, just get the entrypoint
        // at a very basic level.
        let entrypoint = loader_context.get_entrypoint()?
            .ok_or(RunnerError::CannotAcquireEntrypoint)?;

        let (maxstack, maxlocals) = entrypoint.setup_info();

        // Initial Frame Creation
        let mut initial_frame = self
            .stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;

        let code = entrypoint.code();
        let mut constant_fn = |x| loader_context.get_constant(x).ok();

        let mut pc: usize = 0;

        // Keep executing the program until a break condition is met: either a return statement or an
        // error
        loop
        {
            let exec_result =
                exec_instruction(&code[pc..], &mut initial_frame, &mut constant_fn).map_err(RunnerError::ExecutionError)?;

            match exec_result
            {
                InstructionResult::Next =>
                {
                    // Move to next instruction after checking validity
                    (pc + 1 < code.len())
                        .then(|| pc += 1)
                        .ok_or(RunnerError::ProgramCounterOverflow)?;
                }
                InstructionResult::Jump(target) =>
                {
                    // Jump to given target instruction after checking validity
                    (target < code.len())
                        .then(|| pc = target)
                        .ok_or(RunnerError::ProgramCounterOverflow)?;
                }
                InstructionResult::Return(_) =>
                {
                    // Return the required value here?
                    break;
                }
            }
        }

        Ok(())
    }
}
