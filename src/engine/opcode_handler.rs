use crate::{
    engine::{
        opcodes::Opcode,
        stack::{Stack, StackEntry, StackFrame},
    },
    loader::constant_table::{ConstantTable, ConstantTableIndex},
};

#[derive(Debug)]
struct HandlerInputInfo<'a, 'b, 'c>
{
    opcode: u8,
    params: &'a [u8],
    frame: &'b mut StackFrame<'c>,
    constants: &'b ConstantTable<'a>,
}

#[derive(Clone, Copy)]
struct HandlerInfo<'a>
{
    opcode: Opcode,
    param_count: u8,
    handler: &'a dyn Fn(&mut HandlerInputInfo) -> ExecutionResult,
}

#[derive(Clone, Copy)]
pub enum InstructionResult
{
    Next,
    Jump(usize),
    Return,
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutionError
{
    OpcodeNotFound,
    IllegalOpcode,
    MissingParams,
    EmptyStack,
}

type ExecutionResult = Result<InstructionResult, ExecutionError>;

#[expect(
    clippy::panic_in_result_fn,
    reason = "If this invariant check fails, the entire config is malformed"
)]
pub fn exec_instruction<'a>(
    bytecode: &'a [u8],
    frame: &mut StackFrame,
    constants: &ConstantTable<'a>,
) -> ExecutionResult
{
    let (&opcode, operands) = bytecode.split_first().ok_or(ExecutionError::OpcodeNotFound)?;
    let handler_info = HANDLERS.get(opcode as usize).ok_or(ExecutionError::IllegalOpcode)?;

    if operands.len() < handler_info.param_count as usize
    {
        return Err(ExecutionError::MissingParams);
    }

    assert!(
        opcode == handler_info.opcode as u8,
        "HANDLERS Array invalid: misaligned opcode"
    );

    (handler_info.handler)(&mut HandlerInputInfo {
        opcode,
        params: operands,
        frame,
        constants,
    })
}

/*
 * ******************************************************************************
 *                                  HANDLERS
 * ******************************************************************************
 */

// Basic Pushing Handlers

fn push_numeric(input: &mut HandlerInputInfo, value: u64) -> ExecutionResult
{
    input.frame.push(value);
    Ok(InstructionResult::Next)
}

fn push_bytes(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let mut bytes = [0; Stack::ENTRY_SIZE];
    bytes.copy_from_slice(input.params); // If this doesnt copy properly, exec_instruction hasnt done its job properly.

    push_numeric(input, <StackEntry>::from_le_bytes(bytes))
}

#[expect(
    clippy::expect_used,
    reason = "If there aren't enough parameters in the parameters input, this means previous validation steps have failed"
)]
fn push_constant(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let index = <ConstantTableIndex>::from_le_bytes(
        *input
            .params
            .first_chunk()
            .ok_or(ExecutionError::MissingParams)?
    );

    input.constants.push_entry(input.frame, index);
    Ok(InstructionResult::Next)
}

// Basic Loading Handlers

fn load_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    input.frame.push(input.frame.get_local(index as usize));
    Ok(InstructionResult::Next)
}

fn load_local_from_arg(input: &mut HandlerInputInfo) -> ExecutionResult
{
    load_local(input, *input.params.first().ok_or(ExecutionError::MissingParams)?)
}


// Debugging Handlers. Not for actual use

fn simple_print_handler(input: &mut HandlerInputInfo) -> ExecutionResult
{
    println!("{input:?}");
    Ok(InstructionResult::Next)
}

#[expect(
    clippy::panic,
    reason = "This is a debug handler that should never make it to a finished version"
)]
fn unimplemented_handler(_: &mut HandlerInputInfo) -> ExecutionResult
{
    panic!("Opcode not implemented")
}

/*
 * **************************************************************************
 *                               HANDLERS ARRAY
 * **************************************************************************
 */


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

// Is it possible to add any sanity checks into this?
const HANDLERS: [HandlerInfo; u8::MAX as usize + 1] = handlers!(
    { Opcode::Nop, 0, &(|_| Ok(InstructionResult::Next)) }, // nop: Do nothing. [No Change]
    { Opcode::IConst0,       0, push_numeric, 0 },  // i.const.0: Push 0_i64 onto the stack. -> 0
    { Opcode::IConst1,       0, push_numeric, 1 },  // i.const.1: Push 1_i64 onto the stack. -> 1
    { Opcode::IConst2,       0, push_numeric, 2 },  // i.const.2: Push 2_i64 onto the stack. -> 2
    { Opcode::IConst3,       0, push_numeric, 3 },  // i.const.3: Push 3_i64 onto the stack. -> 3
    { Opcode::F4Const0,      0, push_numeric, (0.0_f32).to_bits().into() }, // f4.const.0: Push 0.0f onto the stack. -> 0.0f
    { Opcode::F4Const1,      0, push_numeric, (1.0_f32).to_bits().into() }, // f4.const.1: Push 1.0f onto the stack. -> 1.0f
    { Opcode::F8Const0,      0, push_numeric, (0.0_f64).to_bits() }, // f8.const.0: Push 0.0 onto the stack. -> 0.0
    { Opcode::F8Const1,      0, push_numeric, (1.0_f64).to_bits() }, // f8.const.1: Push 1.0 onto the stack. -> 1.0
    { Opcode::IConst,        1, push_bytes }, // i.const: Push a given 1 byte onto the stack -> [byte]
    { Opcode::IConstW,       2, push_bytes }, // i.const.w: Push a given 2 bytes onto the stack. -> [byte1 << 8 | byte2]
    { Opcode::Const,         4, push_constant }, // const: Push the constant at the given index onto the stack. -> [constant]
    { Opcode::LdArg0,        0, load_local, 0 }, // ld.arg.0: Load the local variable at index 0 onto the stack. -> [local0]
    { Opcode::LdArg1,        0, load_local, 1 }, // ld.arg.1: Load the local variable at index 1 onto the stack. -> [local1]
    { Opcode::LdArg2,        0, load_local, 2 }, // ld.arg.2: Load the local variable at index 2 onto the stack. -> [local2]
    { Opcode::LdArg3,        0, load_local, 3 }, // ld.arg.3: Load the local variable at index 3 onto the stack. -> [local3]
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
    { Opcode::Directive,     0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler }
);
