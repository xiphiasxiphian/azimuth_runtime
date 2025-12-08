pub mod opcode_handler;
pub mod opcodes;
pub mod stack;

use crate::{
    engine::{
        opcode_handler::{ExecutionError, InstructionResult, exec_instruction},
        stack::Stack,
    },
    loader::Loader,
};

#[derive(Debug, Clone, Copy)]
pub enum RunnerError
{
    MissingEntryPoint,
    StackOverflow,
    ExecutionError(ExecutionError),
    ProgramCounterOverflow,
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
        // Get the entry point. This is the "main" function where execution will start
        let entry_point = self.loader.get_entry_point().ok_or(RunnerError::MissingEntryPoint)?;
        let (maxstack, maxlocals) = entry_point.setup_info();

        // Initial Frame Creation and creating the constant table from
        // information provided in the loader
        let mut initial_frame = self
            .stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;

        // Convert the directly parsed constant table into a usable one
        let constant_table = self.loader.get_constant_table();

        let code = entry_point.code();
        let mut pc: usize = 0;

        // Keep executing the program until a break condition is met: either a return statement or an
        // error
        loop
        {
            let exec_result = exec_instruction(&code[pc..], &mut initial_frame, &constant_table)
                .map_err(RunnerError::ExecutionError)?;

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
