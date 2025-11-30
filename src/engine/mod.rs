pub mod opcode_handler;
pub mod opcodes;
pub mod stack;

use crate::{
    engine::{
        opcode_handler::{ExecutionError, InstructionResult, exec_instruction},
        stack::{Stack, StackFrame},
    },
    loader::{Loader, runnable::Runnable},
};

#[derive(Debug, Clone, Copy)]
pub enum RunnerError
{
    MissingEntryPoint,
    StackOverflow,
    ExecutionError(ExecutionError),
    InvalidJump,
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

        let code = entry_point.code();
        let mut pc: usize = 0;

        loop
        {
            let exec_result =
                exec_instruction(&code[pc..], &mut initial_frame).map_err(|x| RunnerError::ExecutionError(x))?;

            match exec_result
            {
                InstructionResult::Next => pc += 1,
                InstructionResult::Jump(target) =>
                {
                    if target >= code.len()
                    {
                        return Err(RunnerError::InvalidJump);
                    }

                    pc = target;
                }
                InstructionResult::Return =>
                {
                    break;
                }
            }
        }

        Ok(())
    }
}
