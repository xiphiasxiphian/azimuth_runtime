use crate::engine::{opcodes::Opcode, stack::StackFrame};

#[derive(Debug)]
struct HandlerInputInfo<'a, 'b, 'c>
{
    opcode: u8,
    params: &'a [u8],
    frame: &'b mut StackFrame<'c>,
}

#[derive(Clone, Copy)]
struct HandlerInfo<'a>
{
    opcode: Opcode,
    param_count: u8,
    handler: &'a dyn Fn(&mut HandlerInputInfo) -> InstructionResult,
}

#[derive(Clone, Copy)]
pub enum InstructionResult
{
    Next,
    Jump(usize),
    Return
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutionError
{
    OpcodeNotFound,
    IllegalOpcode,
}

pub fn exec_instruction<'a, 'b, 'c>(bytecode: &'a [u8], frame: &'b mut StackFrame<'c>) -> Result<InstructionResult, ExecutionError>
{
    let (&opcode, operands) = bytecode.split_first().ok_or(ExecutionError::OpcodeNotFound)?;
    let handler_info = HANDLERS.get(opcode as usize).ok_or(ExecutionError::IllegalOpcode)?;

    assert!(
        opcode == handler_info.opcode as u8,
        "HANDLERS Array invalid: misaligned opcode"
    );

    let instr_result = (handler_info.handler)(&mut HandlerInputInfo {
        opcode,
        params: operands,
        frame,
    });

    Ok(instr_result)
}

fn push_single(input: &mut HandlerInputInfo, value: u32) -> InstructionResult
{
    input.frame.push_single(value);
    InstructionResult::Next
}

fn push_double(input: &mut HandlerInputInfo, value: u64) -> InstructionResult
{
    input.frame.push_double(value);
    InstructionResult::Next
}

// Debugging Handlers. Not for actual use

fn simple_print_handler(input: &mut HandlerInputInfo) -> InstructionResult
{
    println!("{input:?}");
    InstructionResult::Next
}

#[expect(
    clippy::panic,
    reason = "This is a debug handler that should never make it to a finished version"
)]
fn unimplemented_handler(_: &mut HandlerInputInfo) -> InstructionResult
{
    panic!("Opcode not implemented")
}

macro_rules! handlers {
    ($($t:tt),+) => {
        [
            $(
                handler!($t)
            ),+
        ]
    };
}

macro_rules! handler {
    ({$i:expr, $p:expr, $h:ident}) => {
        HandlerInfo { opcode: $i, param_count: $p, handler: &$h }
    };
    ({$i:expr, $p:expr, $h:ident, $($x:expr),+}) => {
        HandlerInfo { opcode: $i, param_count: $p, handler: &(|x| $h(x, $($x),+)) }
    };
    ({$i:expr, $p:expr, $h:expr }) => {
        HandlerInfo { opcode: $i, param_count: $p, handler: $h }
    };
}

const HANDLERS: [HandlerInfo; 256] = handlers!(
    { Opcode::Nop, 0, &(|_| InstructionResult::Next) }, // nop: Do nothing. [No Change]
    { Opcode::I4Const0,      0, push_single, 0 }, // i4.const.0: Push 0 onto the stack. -> 0
    { Opcode::I4Const1,      0, push_single, 1 }, // i4.const.1: Push 1 onto the stack. -> 1
    { Opcode::I4Const2,      0, push_single, 2 }, // i4.const.2: Push 2 onto the stack. -> 2
    { Opcode::I4Const3,      0, push_single, 3 }, // i4.const.3: Push 3 onto the stack. -> 3
    { Opcode::I8Const0,      0, push_double, 0 }, // i8.const.0: Push 0_i64 onto the stack. -> 0
    { Opcode::I8Const1,      0, push_double, 1 }, // i8.const.1: Push 1_i64 onto the stack. -> 1
    { Opcode::I8Const2,      0, push_double, 2 }, // i8.const.2: Push 2_i64 onto the stack. -> 2
    { Opcode::I8Const3,      0, push_double, 3 }, // i8.const.3: Push 3_i64 onto the stack. -> 3
    { Opcode::F4Const0,      0, push_single, (0.0_f32).to_bits() }, // f4.const.0: Push 0.0f onto the stack. -> 0.0f
    { Opcode::F4Const1,      0, push_single, (1.0_f32).to_bits() }, // f4.const.1: Push 1.0f onto the stack. -> 1.0f
    { Opcode::F8Const0,      0, push_double, (0.0_f64).to_bits() }, // f8.const.0: Push 0.0 onto the stack. -> 0.0
    { Opcode::F8Const1,      0, push_double, (1.0_f64).to_bits() }, // f8.const.1: Push 1.0 onto the stack. -> 1.0
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler }
);
