use std::iter::{repeat, repeat_with};

use itertools::Itertools;

use crate::{
    engine::{
        Runner, RunnerError, opcode_handler::{ExecutionError, InstructionResult, exec_instruction}
    },
    guard,
    loader::{FunctionInfo, Loader, LoaderContext},
    memory::stack::{Stack, StackFrame, entry::StackEntry},
};

pub struct ExecutionContext<'a, 'b, 'c>
{
    frame: StackFrame<'c>,
    loader: LoaderContext<'a, 'b>,
}

impl<'a, 'b, 'c> ExecutionContext<'a, 'b, 'c>
where
    'a: 'c,
{
    pub fn run(loader: &'a mut Loader<'b>, stack: &'c mut Stack) -> Result<(), RunnerError>
    {
        let loader_context = loader.initial_context()?;
        let (maxstack, maxlocals, code) = {
            let entrypoint = loader_context
                .get_entrypoint()?
                .ok_or(RunnerError::CannotAcquireEntrypoint)?;

            let (maxstack, maxlocals) = entrypoint.setup_info();
            let code = entrypoint.code();

            (maxstack, maxlocals, code)
        };

        let frame = stack
            .initial_frame(maxlocals, maxstack)
            .ok_or(RunnerError::StackOverflow)?;

        Self {
            frame,
            loader: loader_context,
        }
        .execute_function(code)
        .map(|_| ())
    }

    fn execute_function(&'a mut self, code: &'static [u8]) -> Result<Option<StackEntry>, RunnerError>
    {
        let mut pc: usize = 0;

        // Keep executing the program until a break condition is met: either a return statement or an
        // error
        loop
        {
            let exec_result =
                exec_instruction(&code[pc..], &mut self.frame, |x| self.loader.get_constant(x).ok())?;

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
                InstructionResult::Return(value) =>
                {
                    // Return the required value here?
                    break Ok(value);
                }
                InstructionResult::Invoke(link, func) =>
                {
                    // Split borrows: borrow each field independently
                    let frame_ref = &mut self.frame;
                    let loader_ref = &mut self.loader;

                    loader_ref
                        .with_link(link, |new_loader_context| -> Result<(), RunnerError> {
                            let (maxstack, maxlocals, param_count, code) = {
                                let function_info = new_loader_context.get_function(func)?;
                                let (maxstack, maxlocals) = function_info.setup_info();
                                let code = function_info.code();
                                let param_count = function_info.param_count();

                                (maxstack, maxlocals, param_count, code)
                            };

                            let params: Vec<StackEntry>
                                = repeat_with(|| frame_ref.pop())
                                    .take(param_count.into())
                                    .collect::<Option<Vec<StackEntry>>>()
                                    .ok_or(RunnerError::ExecutionError(ExecutionError::MissingParams))?;

                            let return_value = frame_ref.with_next_frame(maxlocals, maxstack, |mut new_frame| {
                                // Move parameters into local variables
                                for (i, param) in params.into_iter().enumerate()
                                {
                                    let _ = new_frame.set_local(i, param);
                                }

                                let mut new_context = ExecutionContext {
                                    frame: new_frame,
                                    loader: new_loader_context,
                                };

                                new_context.execute_function(code)
                            })?;

                            // If the invoked function returned a value, push it onto the stack
                            if let Some(value) = return_value
                            {
                                frame_ref.push(value).then_some(()).ok_or(RunnerError::StackOverflow)?;
                            }

                            Ok(())
                        })
                        .map_err(|_| RunnerError::LoaderFailure)??;
                }
            }
        }
    }
}
