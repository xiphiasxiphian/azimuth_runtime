pub mod opcode_handler;
pub mod opcodes;

use crate::{
    engine::opcode_handler::{ExecutionError, InstructionResult, exec_instruction},
    loader::{Loader, LoaderError},
    memory::stack::Stack,
};

#[derive(Debug, Clone, Copy)]
pub enum RunnerError
{
    CannotAcquireEntrypoint,
    StackOverflow,
    ExecutionError(ExecutionError),
    ProgramCounterOverflow,
}

pub struct Runner<'a>
{
    stack: &'a mut Stack,
    loader: &'a mut Loader<'a>,
    // heap
}

impl<'a> Runner<'a>
{
    pub fn new(stack: &'a mut Stack, loader: &'a mut Loader<'a>) -> Self
    {
        Self { stack, loader }
    }

    pub fn run(&mut self) -> Result<(), RunnerError>
    {
        // Get the entry point. This is the "main" function where execution will start
        let entry_point = self
            .loader
            .get_entrypoint(todo!())
            .map_err(|_| RunnerError::CannotAcquireEntrypoint)?
            .ok_or(RunnerError::CannotAcquireEntrypoint)?;

        let (maxstack, maxlocals) = entry_point.setup_info();

        // Initial Frame Creation and creating the constant table from
        // information provided in the loader
        let mut initial_frame = self
            .stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;

        // Get constants
        let constant_table = todo!();

        let code = entry_point.code();
        let mut pc: usize = 0;

        // Keep executing the program until a break condition is met: either a return statement or an
        // error
        loop
        {
            let exec_result =
                exec_instruction(&code[pc..], &mut initial_frame, &[]).map_err(RunnerError::ExecutionError)?;

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
