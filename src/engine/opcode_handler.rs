use std::ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not, Rem, Shl, Shr, Sub};

use crate::{
    engine::{
        opcodes::Opcode,
        stack::{Stack, StackEntry, StackFrame}, stackable::Stackable,
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

// Bunch of helper functions to make things a bit cleaner
impl<'a, 'b, 'c> HandlerInputInfo<'a, 'b, 'c>
{
    pub fn stack_pop(&mut self) -> Result<StackEntry, ExecutionError>
    {
        self.frame.pop().ok_or(ExecutionError::EmptyStack)
    }

    pub fn stack_push(&mut self, val: StackEntry) -> Result<(), ExecutionError>
    {
        self.frame.push(val).then_some(()).ok_or(ExecutionError::StackOverflow)
    }

    pub fn local_get(&mut self, index: u8) -> Result<StackEntry, ExecutionError>
    {
        self.frame.get_local(index as usize).ok_or(ExecutionError::IndexOutOfBounds)
    }

    pub fn local_set(&mut self, index: u8, value: StackEntry) -> Result<StackEntry, ExecutionError>
    {
        self.frame.set_local(index as usize, value).ok_or(ExecutionError::IndexOutOfBounds)
    }

    /// Helper function for pulling a given number of parameters out of the bytecode stream.
    /// This function will fail if there aren't enough parameters, returning and Err(_)
    fn pull_params(&self, count: usize) -> Result<&[u8], ExecutionError>
    {
        self.params
            .split_at_checked(count)
            .map(|(x, _)| x)
            .ok_or(ExecutionError::MissingParams)
    }

    fn stack_pop_many<const N: usize>(&mut self) -> Result<[u64; N], ExecutionError>
    {
        let mut values = [0; N];
        for i in 0..N
        {
            values[i] = self.stack_pop()?
        }

        Ok(values)
    }
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
    IllegalParam,
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
fn push_numeric(input: &mut HandlerInputInfo, value: u64) -> ExecutionResult
{
    input
        .stack_push(value)
        .map(|_| InstructionResult::Next)
}

/// Push bytes found from parameters onto the stack
///
/// The number of bytes must be less than `Stack::ENTRY_SIZE`
fn push_bytes(input: &mut HandlerInputInfo) -> ExecutionResult
{
    // Ensures that the number of bytes provided will actually fit
    // within a stack entry
    if input.params.len() <= Stack::ENTRY_SIZE { return Err(ExecutionError::IllegalParam) }

    let mut bytes = [0; Stack::ENTRY_SIZE]; // This is set to the stack entry size.
    bytes[0..(input.params.len())].copy_from_slice(input.params);

    // Defer to just pushing a normal numeric value
    push_numeric(input, <StackEntry>::from_le_bytes(bytes))
}

/// Gets a constant from the constant table and pushes it to the stack.
fn push_constant(input: &mut HandlerInputInfo) -> ExecutionResult
{
    // Construct the constant table index from the given parameters.
    let bytes = input.pull_params(4)?.first_chunk().ok_or(ExecutionError::MissingParams)?;
    let index = <ConstantTableIndex>::from_le_bytes(*bytes);

    // Copy the constant from the constant table onto the stack.
    // This function will take care of the differing behaviours depending on
    // the type of constant
    input
        .constants
        .push_entry(input.frame, index)
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
    input.stack_pop()
        .map(|_| InstructionResult::Next) // Discard value
}

/// Duplicates the value on top of the stack.
fn dup(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let value = input.frame.peek().ok_or(ExecutionError::EmptyStack)?;
    push_numeric(input, *value)
}

/// Swaps the top 2 stack values
fn swap(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let value1 = input.stack_pop()?;
    let value2 = input.stack_pop()?;

    input.stack_push(value1)
        .and_then(|_| input.stack_push(value2))
        .map(|_| InstructionResult::Next)
}

// Basic Local Variable Handlers

/// Loads a local variable at the provided index onto the stack
fn load_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    let val = input.local_get(index)?;
    input.stack_push(val)
        .map(|_| InstructionResult::Next)
}

/// Stores the value on top of the stack onto the stack
fn store_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    let value = input.stack_pop()?;
    input.local_set(index, value)
        .map(|_| InstructionResult::Next)
}

// Arithmetic Handlers

fn unaryop<T, F>(input: &mut HandlerInputInfo, op: F) -> ExecutionResult
where
    T: Stackable,
    F: Fn(T) -> T
{
    let value = input.stack_pop().map(T::from_entry)?;
    input.stack_push(op(value).into_entry())
        .map(|_| InstructionResult::Next)
}

fn binop<T, F>(input: &mut HandlerInputInfo, op: F) -> ExecutionResult
where
    T: Stackable,
    F: Fn(T, T) -> T
{
    let [value1, value2] = input.stack_pop_many::<2>()?.map(T::from_entry);
    input.stack_push(op(value1, value2).into_entry())
        .map(|_| InstructionResult::Next)
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
    { Opcode::Nop,           0, &(|_| Ok(InstructionResult::Next)) },
    { Opcode::IConst0,       0, push_numeric, 0 },
    { Opcode::IConst1,       0, push_numeric, 1 },
    { Opcode::IConst2,       0, push_numeric, 2 },
    { Opcode::IConst3,       0, push_numeric, 3 },
    { Opcode::F4Const0,      0, push_numeric, (0.0_f32).to_bits().into() },
    { Opcode::F4Const1,      0, push_numeric, (1.0_f32).to_bits().into() },
    { Opcode::F8Const0,      0, push_numeric, (0.0_f64).to_bits() },
    { Opcode::F8Const1,      0, push_numeric, (1.0_f64).to_bits() },
    { Opcode::IConst,        1, push_bytes },
    { Opcode::IConstW,       2, push_bytes },
    { Opcode::Const,         4, push_constant },
    { Opcode::LdArg0,        0, load_local, 0 },
    { Opcode::LdArg1,        0, load_local, 1 },
    { Opcode::LdArg2,        0, load_local, 2 },
    { Opcode::LdArg3,        0, load_local, 3 },
    { Opcode::LdArg,         1, &(|x| load_local(x, x.pull_params(1)?[0])) },
    { Opcode::StArg0,        0, store_local, 0 },
    { Opcode::StArg1,        0, store_local, 1 },
    { Opcode::StArg2,        0, store_local, 2 },
    { Opcode::StArg3,        0, store_local, 3 },
    { Opcode::StArg,         1, &(|x| store_local(x, x.pull_params(1)?[0])) },
    { Opcode::Pop,           0, pop },
    { Opcode::Dup,           0, dup },
    { Opcode::Swap,          0, swap },
    { Opcode::Ret,           0, &(|_| Ok(InstructionResult::Return(false))) },
    { Opcode::RetVal,        0, &(|_| Ok(InstructionResult::Return(true))) },
    { Opcode::IAdd,          0, binop, <u64>::add },
    { Opcode::F4Add,         0, binop, <f32>::add },
    { Opcode::F8Add,         0, binop, <f64>::add },
    { Opcode::ISub,          0, binop, <u64>::sub }, // TODO: Think about wrapping
    { Opcode::F4Sub,         0, binop, <f32>::sub },
    { Opcode::F8Sub,         0, binop, <f64>::sub },
    { Opcode::IMul,          0, binop, <u64>::mul },
    { Opcode::F4Mul,         0, binop, <f32>::mul },
    { Opcode::F8Mul,         0, binop, <f64>::mul },
    { Opcode::IDiv,          0, binop, <u64>::div },
    { Opcode::F4Div,         0, binop, <f32>::div },
    { Opcode::F8Div,         0, binop, <f64>::div },
    { Opcode::IRem,          0, binop, <u64>::rem },
    { Opcode::F4Rem,         0, binop, <f32>::rem },
    { Opcode::F8Rem,         0, binop, <f64>::rem },
    { Opcode::INeg,          0, unaryop, <i64>::neg },
    { Opcode::F4Neg,         0, unaryop, <f32>::neg },
    { Opcode::F8Neg,         0, unaryop, <f64>::neg },
    { Opcode::Shl,           0, binop, <u64>::shl },
    { Opcode::Shr,           0, binop, <u64>::shr },
    { Opcode::AShr,          0, binop, <i64>::shr },
    { Opcode::And,           0, binop, <u64>::bitand },
    { Opcode::Or,            0, binop, <u64>::bitor },
    { Opcode::Xor,           0, binop, <u64>::bitxor },
    { Opcode::Not,           0, unaryop, <u64>::not },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
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
