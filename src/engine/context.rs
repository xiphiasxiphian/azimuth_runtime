use crate::{engine::{RunnerError, opcode_handler::{InstructionResult, exec_instruction}}, loader::{FunctionInfo, Loader, LoaderContext}, memory::stack::{Stack, StackFrame, entry::StackEntry}};



pub struct ExecutionContext<'a, 'b, 'c>
{
    frame: StackFrame<'c>,
    loader: LoaderContext<'a, 'b>,
}

impl<'a, 'b, 'c> ExecutionContext<'a, 'b, 'c>
where
    'a: 'c
{
    pub fn run(loader: &'a mut Loader<'b>, stack: &'c mut Stack) -> Result<(), RunnerError>
    {
        let loader_context = loader.initial_context()?;
        let (maxstack, maxlocals, code) = {
            let entrypoint = loader_context.get_entrypoint()?
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
        }.execute_function(code).map(|_| ())
    }

    fn execute_function(&'a mut self, code: &'static [u8]) -> Result<Option<StackEntry>, RunnerError>
    {
        let mut constant_fn = |x| self.loader.get_constant(x).ok();
        let mut pc: usize = 0;

        // Keep executing the program until a break condition is met: either a return statement or an
        // error
        loop
        {
            let exec_result =
                exec_instruction(&code[pc..], &mut self.frame, &mut constant_fn).map_err(RunnerError::ExecutionError)?;

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
                },
                InstructionResult::Invoke(link, func) =>
                {
                    self.loader.with_link(link, |cont| -> Result<(), RunnerError> {
                        let (maxstack, maxlocals, code) = {
                            let entrypoint = cont.get_function(func)?;

                            let (maxstack, maxlocals) = entrypoint.setup_info();
                            let code = entrypoint.code();

                            (maxstack, maxlocals, code)
                        };

                        self.frame.with_next_frame(
                            maxlocals,
                            maxstack,
                            |frame| {
                                Self {
                                    frame,
                                    loader: cont,
                                }.execute_function(code);
                            }
                        );

                        Ok(())
                    });
                }
            }
        }
    }
}
