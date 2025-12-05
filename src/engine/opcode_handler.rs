use crate::{
    engine::{
        opcodes::Opcode,
        stack::{Stack, StackEntry, StackFrame},
    },
    loader::constant_table::{ConstantTable, ConstantTableIndex},
};

/// Contains information given to each instruction handler
///
/// ### Fields
/// `opcode` - The numerical value of the opcode
///
/// `params` - A slice of the parameters passed into this opcode
///
/// `frame` - A reference to the current stack frame
///
/// `constants` - A reference to the constant table
///
/// ### Note
/// The lifetime parameters of this struct reflect the expected lifetimes of the references:
/// the `params` slice will have the same lifetime as the contents of the constant table (`'a`),
/// as they will both be stored within the loader's metaspace. The reference to the stack frame
/// and the reference to the constant table will both be the same as they are both
/// constructed in the loader
#[derive(Debug)]
struct HandlerInputInfo<'a, 'b, 'c>
{
    opcode: u8,
    params: &'a [u8],
    frame: &'b mut StackFrame<'c>,
    constants: &'b ConstantTable<'a>,
}

/// Information about a handler for a given instruction
///
/// ## Fields
/// `opcode` - The opcode this handler is responsible for. This is mainly used for validation
///
/// `param_count` - The number of bytes this handler takes as parameters
///
/// `handler` - The function that handles the given opcode
///
/// ## Note
/// This type should remain a copy type
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
    Return(bool),
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutionError
{
    OpcodeNotFound,
    IllegalOpcode,
    MissingParams,
    EmptyStack,
    StackOverflow,
    IndexOutOfBounds,
}

type ExecutionResult = Result<InstructionResult, ExecutionError>;

/// Executes the next instruction found from the sequence of bytes.
///
/// Takes the current stream of bytcode, the current stack frame and the
/// constant table associated with this bytecode stream.
/// It is expected that the first byte in the `bytecode` slice will be
/// the opcode, and then the remaining bytes can be whatever is next in the stream.
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
    // Get the bytecode out of the stream. As this is "user input", it is critical
    // at all stages to check whether there are actually enough values in the stream
    // to meet expectations
    let (&opcode, operands) = bytecode.split_first().ok_or(ExecutionError::OpcodeNotFound)?;
    let handler_info = HANDLERS.get(opcode as usize).ok_or(ExecutionError::IllegalOpcode)?;

    if operands.len() < handler_info.param_count as usize
    {
        return Err(ExecutionError::MissingParams);
    }

    // If this assertion fails, this means that the HANDLERS table has been malformed
    // and found handler doesn't match the opcode.
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

/// Helper function for pulling a given number of parameters out of the bytecode stream.
/// This function will fail if there aren't enough parameters, returning and Err(_)
fn pull_params<const N: usize>(input: &[u8]) -> Result<[u8; N], ExecutionError>
{
    Ok(*input.first_chunk().ok_or(ExecutionError::MissingParams)?)
}

/*
 * ******************************************************************************
 *                                  HANDLERS
 * ******************************************************************************
 */

// Basic Stack Handlers


/// Push a given number (in the form of `u64`) onto the stack.
///
/// It is expected that any other numeric type (such as `f32` of `f64`) must be converted
/// into a `u64` format. This can normally be done with `.to_bits()`.
/// These bits can later be recovered into their original type.
#[expect(clippy::unnecessary_wraps, reason = "Needs to conform to handler format")]
fn push_numeric(input: &mut HandlerInputInfo, value: u64) -> ExecutionResult
{
    input.frame.push(value)
        .then_some(InstructionResult::Next)
        .ok_or(ExecutionError::StackOverflow)
}

/// Push bytes found from parameters onto the stack
///
/// The number of bytes must be less than `Stack::ENTRY_SIZE`
fn push_bytes(input: &mut HandlerInputInfo) -> ExecutionResult
{
    // Ensures that the number of bytes provided will actually fit
    // within a stack entry
    assert!(input.params.len() <= Stack::ENTRY_SIZE);

    let mut bytes = [0; Stack::ENTRY_SIZE]; // This is set to the stack entry size.
    bytes[0..(input.params.len())].copy_from_slice(input.params);

    // Defer to just pushing a normal numeric value
    push_numeric(input, <StackEntry>::from_le_bytes(bytes))
}

/// Gets a constant from the constant table and pushes it to the stack.
fn push_constant(input: &mut HandlerInputInfo) -> ExecutionResult
{
    // Construct the constant table index from the given parameters.
    let index = <ConstantTableIndex>::from_le_bytes(pull_params(input.params)?);

    // Copy the constant from the constant table onto the stack.
    // This function will take care of the differing behaviours depending on
    // the type of constant
    input.constants.push_entry(input.frame, index)
        .ok_or(ExecutionError::IndexOutOfBounds)?
        .then_some(InstructionResult::Next)
        .ok_or(ExecutionError::StackOverflow)
}

/// Pops a value off the stack, explicitly discarding it
///
/// This should only be used to remove redundant values off the stack,
/// as it throws away whatever the value it found was.
fn pop(input: &mut HandlerInputInfo) -> ExecutionResult
{
    input
        .frame
        .pop()
        .ok_or(ExecutionError::EmptyStack)
        .map(|_| InstructionResult::Next) // Discard whatever the value was
}


/// Duplicates the value on top of the stack.
fn dup(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let value = input.frame.peek().ok_or(ExecutionError::EmptyStack)?;
    push_numeric(input, *value)
}

// Basic Local Variable Handlers

/// Loads a local variable at the provided index onto the stack
fn load_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    input.frame.push(
        input.frame
            .get_local(index as usize)
            .ok_or(ExecutionError::IndexOutOfBounds)?
    )
    .then_some(InstructionResult::Next)
    .ok_or(ExecutionError::StackOverflow)
}

/// Stores the value on top of the stack onto the stack
fn store_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    let value = input.frame.pop().ok_or(ExecutionError::EmptyStack)?;
    input.frame.set_local(index as usize, value)
        .map(|_| InstructionResult::Next)
        .ok_or(ExecutionError::IndexOutOfBounds)
}

// Debugging Handlers. Not for actual use

#[expect(
    clippy::panic_in_result_fn,
    reason = "This is a debug handler that should never make it to a finished version"
)]
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
    { Opcode::Nop,           0, &(|_| Ok(InstructionResult::Next)) }, // nop: Do nothing. [No Change]
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
    { Opcode::LdArg,         1, &(|x| load_local(x, pull_params::<1>(x.params)?[0])) }, // ld.arg: Load local variable to the stack. -> [local{index}]
    { Opcode::StArg0,        0, store_local, 0 }, // st.arg.0: Store top of the stack into local variable 0. [value] ->
    { Opcode::StArg1,        0, store_local, 1 }, // st.arg.1: Store top of the stack into local variable 1. [value] ->
    { Opcode::StArg2,        0, store_local, 2 }, // st.arg.2: Store top of the stack into local variable 2. [value] ->
    { Opcode::StArg3,        0, store_local, 3 }, // st.arg.3: Store top of the stack into local variable 3. [value] ->
    { Opcode::StArg,         1, &(|x| store_local(x, pull_params::<1>(x.params)?[0])) }, // st.arg: Store top of the stack into local variable. [value] ->
    { Opcode::Pop,           0, pop }, // pop: Discard the top of the stack. [value] ->
    { Opcode::Dup,           0, dup }, // dup: Duplicate the value on the top of the stack [value] -> [value], [value]
    { Opcode::Ret,           0, &(|_| Ok(InstructionResult::Return(false))) }, // ret: Return out of the current function. -> !
    { Opcode::RetVal,        0, &(|_| Ok(InstructionResult::Return(true))) }, // ret.val: Return with the value top of hte stack. [value] -> !
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
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
