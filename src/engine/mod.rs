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
        let entry_point = self.loader.get_entry_point().ok_or(RunnerError::MissingEntryPoint)?;
        let (maxstack, maxlocals) = entry_point.setup_info();

        let mut initial_frame = self
            .stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;

        let constant_table = self.loader.get_constant_table();

        let code = entry_point.code();
        let mut pc: usize = 0;

        loop
        {
            let exec_result = exec_instruction(&code[pc..], &mut initial_frame, &constant_table)
                .map_err(|x| RunnerError::ExecutionError(x))?;

            match exec_result
            {
                InstructionResult::Next =>
                {
                    (pc + 1 < code.len())
                        .then(|| pc += 1)
                        .ok_or(RunnerError::ProgramCounterOverflow)?;
                }
                InstructionResult::Jump(target) =>
                {
                    (target < code.len())
                        .then(|| pc = target)
                        .ok_or(RunnerError::ProgramCounterOverflow)?;
                }
                InstructionResult::Return =>
                {
                    // Return the required value here? How would that work to say it could be multiple types?
                    break;
                }
            }
        }

        Ok(())
    }
}
