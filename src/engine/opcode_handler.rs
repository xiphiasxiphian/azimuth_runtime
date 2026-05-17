use std::ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not, Rem, Shl, Shr, Sub};

use num_traits::FromBytes;

use crate::{
    engine::opcodes::Opcode,
    guard,
    memory::{
        datumspace::tables::constant_table::{Constant, ConstantTableIndex},
        stack::{Stack, StackFrame, convert::StackableConvert, entry::StackEntry},
    },
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
struct HandlerInputInfo<'a, 'b, 'c>
{
    opcode: u8,
    params: &'a [u8],
    frame: &'b mut StackFrame<'c>,
    constants: &'b mut (dyn FnMut(usize) -> Option<Constant> + 'b),
}

// Bunch of helper functions to make things a bit cleaner
impl HandlerInputInfo<'_, '_, '_>
{
    pub fn stack_pop(&mut self) -> Result<StackEntry, ExecutionError>
    {
        self.frame.pop().ok_or(ExecutionError::EmptyStack)
    }

    pub fn stack_push(&mut self, val: StackEntry) -> Result<(), ExecutionError>
    {
        self.frame.push(val).then_some(()).ok_or(ExecutionError::StackOverflow)
    }

    pub fn local_get(&mut self, index: u8) -> Result<&StackEntry, ExecutionError>
    {
        self.frame
            .get_local(index as usize)
            .ok_or(ExecutionError::IndexOutOfBounds)
    }

    pub fn local_set(&mut self, index: u8, value: StackEntry) -> Result<StackEntry, ExecutionError>
    {
        self.frame
            .set_local(index as usize, value)
            .ok_or(ExecutionError::IndexOutOfBounds)
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

    fn get_numeric<T, const N: usize>(&self, start: usize) -> Result<T, ExecutionError>
    where
        T: FromBytes<Bytes = [u8; N]>,
    {
        self.params
            .split_at_checked(start)
            .and_then(|(_, x)| Some(<T>::from_le_bytes(x.first_chunk()?)))
            .ok_or(ExecutionError::MissingParams)
    }

    fn stack_pop_many<const N: usize>(&mut self) -> Result<[StackEntry; N], ExecutionError>
    {
        let mut values = [StackEntry::Unsigned(0); N];
        for val in &mut values
        {
            *val = self.stack_pop()?;
        }

        Ok(values)
    }

    fn move_constant(&mut self, index: ConstantTableIndex) -> Result<(), ExecutionError>
    {
        (self.constants)(<usize>::try_from(index).map_err(|_| ExecutionError::IndexOutOfBounds)?)
            .ok_or(ExecutionError::IndexOutOfBounds)
            .and_then(|x| self.stack_push((x).into()))
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
    Offset(u16),
    Return(Option<StackEntry>),
    Invoke(usize, usize),
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
    TypeMismatch,
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
pub fn exec_instruction(
    bytecode: &'static [u8],
    frame: &mut StackFrame,
    mut constants: impl FnMut(usize) -> Option<Constant>,
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
        constants: &mut constants,
    })
}

/*
 * ******************************************************************************
 *                                  HANDLERS
 * ******************************************************************************
 */

// Basic Stack Handlers

/// Push a given number (in the form of `u64`) onto the stack.
fn push_numeric<T>(input: &mut HandlerInputInfo, value: T) -> ExecutionResult
where
    T: Into<StackEntry>,
{
    input.stack_push(value.into()).map(|()| InstructionResult::Next)
}

/// Push bytes found from parameters onto the stack
///
/// The number of bytes must be less than `Stack::ENTRY_SIZE`, as
/// that is the max size of an integer
fn push_bytes<T>(input: &mut HandlerInputInfo) -> ExecutionResult
where
    T: Into<StackEntry> + FromBytes<Bytes = [u8; Stack::ENTRY_SIZE]>,
{
    // Ensures that the number of bytes provided will actually fit
    // within a stack entry
    guard!(input.params.len() <= Stack::ENTRY_SIZE, ExecutionError::IllegalParam);

    let mut bytes = [0; Stack::ENTRY_SIZE]; // This is set to the stack entry size.
    bytes[0..(input.params.len())].copy_from_slice(input.params);

    // Defer to just pushing a normal numeric value
    push_numeric(input, <T>::from_le_bytes(&bytes))
}

/// Gets a constant from the constant table and pushes it to the stack.
fn push_constant(input: &mut HandlerInputInfo) -> ExecutionResult
{
    const SIZE: usize = size_of::<ConstantTableIndex>();

    // Construct the constant table index from the given parameters.
    let bytes = input
        .pull_params(size_of::<ConstantTableIndex>())?
        .first_chunk::<SIZE>()
        .ok_or(ExecutionError::MissingParams)?;
    let index = <ConstantTableIndex>::from_le_bytes(*bytes);

    // Copy the constant from the constant table onto the stack.
    // This function will take care of the differing behaviours depending on
    // the type of constant
    input.move_constant(index).map(|()| InstructionResult::Next)
}

/// Pops a value off the stack, explicitly discarding it
///
/// This should only be used to remove redundant values off the stack,
/// as it throws away whatever the value it found was.
fn pop(input: &mut HandlerInputInfo) -> ExecutionResult
{
    input.stack_pop().map(|_| InstructionResult::Next) // Discard value
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

    input
        .stack_push(value1)
        .and_then(|()| input.stack_push(value2))
        .map(|()| InstructionResult::Next)
}

/// Returns from a function, optionally with a value
fn ret(input: &mut HandlerInputInfo, with_value: bool) -> ExecutionResult
{
    Ok(InstructionResult::Return(
        with_value.then(|| input.stack_pop()).transpose()?,
    ))
}

// Basic Local Variable Handlers

/// Loads a local variable at the provided index onto the stack
fn load_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    let val = *input.local_get(index)?;
    input.stack_push(val).map(|()| InstructionResult::Next)
}

/// Stores the value on top of the stack onto the stack
fn store_local(input: &mut HandlerInputInfo, index: u8) -> ExecutionResult
{
    let value = input.stack_pop()?;
    input.local_set(index, value).map(|_| InstructionResult::Next)
}

// Arithmetic Handlers

fn unaryop<F>(input: &mut HandlerInputInfo, op: F) -> ExecutionResult
where
    F: Fn(StackEntry) -> Option<StackEntry>,
{
    let value = input.stack_pop()?;
    input
        .stack_push(op(value).ok_or(ExecutionError::TypeMismatch)?)
        .map(|()| InstructionResult::Next)
}

fn binop<F>(input: &mut HandlerInputInfo, op: F) -> ExecutionResult
where
    F: Fn(StackEntry, StackEntry) -> Option<StackEntry>,
{
    let [value1, value2] = input.stack_pop_many::<2>()?;
    input
        .stack_push(op(value1, value2).ok_or(ExecutionError::TypeMismatch)?)
        .map(|()| InstructionResult::Next)
}

// Conversion

fn convert<I, O>(input: &mut HandlerInputInfo) -> ExecutionResult
where
    I: TryFrom<StackEntry>,
    O: Into<StackEntry> + StackableConvert<I>,
{
    let value = input.stack_pop()?;
    input
        .stack_push(value.cast::<I, O>().ok_or(ExecutionError::TypeMismatch)?)
        .map(|()| InstructionResult::Next)
}

// Conditionals

fn branch<F, const N: usize>(input: &mut HandlerInputInfo, condition: F) -> ExecutionResult
where
    F: FnOnce([StackEntry; N]) -> bool
{
    let test_values = input.stack_pop_many()?;

    if !condition(test_values) { return Ok(InstructionResult::Next) }

    // Get branch offset
    let offset: u16 = input.get_numeric(0)?;
    Ok(InstructionResult::Offset(offset))
}


// Functions

fn invoke(input: &mut HandlerInputInfo) -> ExecutionResult
{
    let link_index: u32 = input.get_numeric(0)?;
    let func: u32 = input.get_numeric(size_of::<u32>())?;
    // let symbol_id = SymbolId(
    //     input.params
    //     .get(1..size_of::<SymbolId>())
    //     .ok_or(ExecutionError::MissingParams)
    //     .and_then(|x| x.try_into().map_err(|_| ExecutionError::IllegalParam))?
    // );

    Ok(InstructionResult::Invoke(
        link_index.try_into().expect("Running on sub 32-bit machine"),
        func.try_into().expect("Running on sub 32-bit machine"),
    ))
}

// Debugging Handlers. Not for actual use

#[expect(
    clippy::panic_in_result_fn,
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
    { Opcode::IConst0,       0, push_numeric, 0_u64 },
    { Opcode::IConst1,       0, push_numeric, 1_u64 },
    { Opcode::IConst2,       0, push_numeric, 2_u64 },
    { Opcode::IConst3,       0, push_numeric, 3_u64 },
    { Opcode::F4Const0,      0, push_numeric, 0.0_f32 },
    { Opcode::F4Const1,      0, push_numeric, 1.0_f32 },
    { Opcode::F8Const0,      0, push_numeric, 0.0_f64 },
    { Opcode::F8Const1,      0, push_numeric, 1.0_f64 },
    { Opcode::IConst,        1, &(|x| push_bytes::<i64>(x)) },
    { Opcode::IConstW,       2, &(|x| push_bytes::<u64>(x)) },
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
    { Opcode::Ret,           0, ret, false },
    { Opcode::RetVal,        0, ret, true },
    { Opcode::Add,           0, binop, Add::add },
    { Opcode::Sub,           0, binop, Sub::sub },
    { Opcode::Mul,           0, binop, Mul::mul },
    { Opcode::Div,           0, binop, Div::div },
    { Opcode::Rem,           0, binop, Rem::rem },
    { Opcode::Neg,           0, unaryop, Neg::neg },
    { Opcode::Shl,           0, binop, Shl::shl },
    { Opcode::Shr,           0, binop, Shr::shr },
    { Opcode::And,           0, binop, BitAnd::bitand },
    { Opcode::Or,            0, binop, BitOr::bitor },
    { Opcode::Xor,           0, binop, BitXor::bitxor },
    { Opcode::Not,           0, unaryop, Not::not },
    { Opcode::IConvertF4,    0, &(|x| convert::<i64, f32>(x)) }, // Using i64 to avoid sign loss
    { Opcode::IConvertF8,    0, &(|x| convert::<i64, f64>(x)) },
    { Opcode::F4ConvertI,    0, &(|x| convert::<f32, i64>(x)) },
    { Opcode::F4ConvertF8,   0, &(|x| convert::<f32, f64>(x)) },
    { Opcode::F8ConvertI,    0, &(|x| convert::<f64, i64>(x)) },
    { Opcode::F8ConvertF4,   0, &(|x| convert::<f64, f32>(x)) },
    { Opcode::Invoke,        8, invoke },
    { Opcode::IfEq,          2, branch, |[y]| y == StackEntry::Unsigned(0) },
    { Opcode::IfNe,          2, branch, |[y]| y != StackEntry::Unsigned(0) },
    { Opcode::IfLt,          2, branch, |[y]| y < StackEntry::Unsigned(0) },
    { Opcode::IfGe,          2, branch, |[y]| y >= StackEntry::Unsigned(0) },
    { Opcode::IfGt,          2, branch, |[y]| y > StackEntry::Unsigned(0) },
    { Opcode::IfLe,          2, branch, |[y]| y <= StackEntry::Unsigned(0) },
    { Opcode::IfEqCmp,       2, branch, |[a, b]| a == b },
    { Opcode::IfNeCmp,       2, branch, |[a, b]| a != b },
    { Opcode::IfLtCmp,       2, branch, |[a, b]| a < b },
    { Opcode::IfGeCmp,       2, branch, |[a, b]| a >= b },
    { Opcode::IfGtCmp,       2, branch, |[a, b]| a > b},
    { Opcode::IfLeCmp,       2, branch, |[a, b]| a <= b },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
    { Opcode::Unimplemented, 0, unimplemented_handler },
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
