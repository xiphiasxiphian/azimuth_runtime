use crate::engine::stack::StackFrame;

#[derive(Debug,)]
struct HandlerInputInfo<'a,>
{
    opcode: u8,
    params: &'a [u8],
    frame: &'a mut StackFrame<'a,>,
}

#[derive(Clone, Copy,)]
struct HandlerInfo<'a,>
{
    opcode: u8,
    param_count: u8,
    handler: &'a dyn Fn(HandlerInputInfo,) -> Option<usize,>,
}

pub fn exec_instruction(bytecode: &[u8], pc: usize, frame: &mut StackFrame,) -> usize
{
    let opcode = bytecode[pc];
    let handler_info = &HANDLERS[opcode as usize];

    assert!(
        opcode == handler_info.opcode,
        "HANDLERS Array invalid: misaligned opcode"
    );
    let new_pc = pc + handler_info.param_count as usize + 1;

    (handler_info.handler)(HandlerInputInfo {
        opcode,
        params: &bytecode[(pc + 1)..new_pc],
        frame,
    },)
    .unwrap_or(new_pc,)
}

fn push_single(input: HandlerInputInfo, value: u32,) -> Option<usize,>
{
    input.frame.push_single(value,);
    None
}

fn push_double(input: HandlerInputInfo, value: u64,) -> Option<usize,>
{
    input.frame.push_double(value,);
    None
}

// Debugging Handlers. Not for actual use

fn simple_print_handler(input: HandlerInputInfo,) -> Option<usize,>
{
    dbg!(input);
    None
}

fn unimplemented_handler(input: HandlerInputInfo,) -> Option<usize,>
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
    { 0, 0, &(|_| None) }, // nop: Do nothing. [No Change]
    { 1, 0, push_single, 0 }, // i4.const.0: Push 0 onto the stack. -> 0
    { 2, 0, push_single, 1 }, // i4.const.1: Push 1 onto the stack. -> 1
    { 3, 0, push_single, 2 }, // i4.const.2: Push 2 onto the stack. -> 2
    { 4, 0, push_single, 3 }, // i4.const.3: Push 3 onto the stack. -> 3
    { 5, 0, push_double, 0 }, // i8.const.0: Push 0_i64 onto the stack. -> 0
    { 6, 0, push_double, 1 }, // i8.const.1: Push 1_i64 onto the stack. -> 1
    { 7, 0, push_double, 2 }, // i8.const.2: Push 2_i64 onto the stack. -> 2
    { 8, 0, push_double, 3 }, // i8.const.3: Push 3_i64 onto the stack. -> 3
    { 9, 0, push_single, (0.0_f32).to_bits() }, // f4.const.0: Push 0.0f onto the stack. -> 0.0f
    { 10, 0, push_single, (1.0_f32).to_bits() }, // f4.const.1: Push 1.0f onto the stack. -> 1.0f
    { 11, 0, push_double, (0.0_f64).to_bits() }, // f8.const.0: Push 0.0 onto the stack. -> 0.0
    { 12, 0, push_double, (1.0_f64).to_bits() }, // f8.const.1: Push 1.0 onto the stack. -> 1.0
    { 13, 0, unimplemented_handler },
    { 14, 0, unimplemented_handler },
    { 15, 0, unimplemented_handler },
    { 16, 0, unimplemented_handler },
    { 17, 0, unimplemented_handler },
    { 18, 0, unimplemented_handler },
    { 19, 0, unimplemented_handler },
    { 20, 0, unimplemented_handler },
    { 21, 0, unimplemented_handler },
    { 22, 0, unimplemented_handler },
    { 23, 0, unimplemented_handler },
    { 24, 0, unimplemented_handler },
    { 25, 0, unimplemented_handler },
    { 26, 0, unimplemented_handler },
    { 27, 0, unimplemented_handler },
    { 28, 0, unimplemented_handler },
    { 29, 0, unimplemented_handler },
    { 30, 0, unimplemented_handler },
    { 31, 0, unimplemented_handler },
    { 32, 0, unimplemented_handler },
    { 33, 0, unimplemented_handler },
    { 34, 0, unimplemented_handler },
    { 35, 0, unimplemented_handler },
    { 36, 0, unimplemented_handler },
    { 37, 0, unimplemented_handler },
    { 38, 0, unimplemented_handler },
    { 39, 0, unimplemented_handler },
    { 40, 0, unimplemented_handler },
    { 41, 0, unimplemented_handler },
    { 42, 0, unimplemented_handler },
    { 43, 0, unimplemented_handler },
    { 44, 0, unimplemented_handler },
    { 45, 0, unimplemented_handler },
    { 46, 0, unimplemented_handler },
    { 47, 0, unimplemented_handler },
    { 48, 0, unimplemented_handler },
    { 49, 0, unimplemented_handler },
    { 50, 0, unimplemented_handler },
    { 51, 0, unimplemented_handler },
    { 52, 0, unimplemented_handler },
    { 53, 0, unimplemented_handler },
    { 54, 0, unimplemented_handler },
    { 55, 0, unimplemented_handler },
    { 56, 0, unimplemented_handler },
    { 57, 0, unimplemented_handler },
    { 58, 0, unimplemented_handler },
    { 59, 0, unimplemented_handler },
    { 60, 0, unimplemented_handler },
    { 61, 0, unimplemented_handler },
    { 62, 0, unimplemented_handler },
    { 63, 0, unimplemented_handler },
    { 64, 0, unimplemented_handler },
    { 65, 0, unimplemented_handler },
    { 66, 0, unimplemented_handler },
    { 67, 0, unimplemented_handler },
    { 68, 0, unimplemented_handler },
    { 69, 0, unimplemented_handler },
    { 70, 0, unimplemented_handler },
    { 71, 0, unimplemented_handler },
    { 72, 0, unimplemented_handler },
    { 73, 0, unimplemented_handler },
    { 74, 0, unimplemented_handler },
    { 75, 0, unimplemented_handler },
    { 76, 0, unimplemented_handler },
    { 77, 0, unimplemented_handler },
    { 78, 0, unimplemented_handler },
    { 79, 0, unimplemented_handler },
    { 80, 0, unimplemented_handler },
    { 81, 0, unimplemented_handler },
    { 82, 0, unimplemented_handler },
    { 83, 0, unimplemented_handler },
    { 84, 0, unimplemented_handler },
    { 85, 0, unimplemented_handler },
    { 86, 0, unimplemented_handler },
    { 87, 0, unimplemented_handler },
    { 88, 0, unimplemented_handler },
    { 89, 0, unimplemented_handler },
    { 90, 0, unimplemented_handler },
    { 91, 0, unimplemented_handler },
    { 92, 0, unimplemented_handler },
    { 93, 0, unimplemented_handler },
    { 94, 0, unimplemented_handler },
    { 95, 0, unimplemented_handler },
    { 96, 0, unimplemented_handler },
    { 97, 0, unimplemented_handler },
    { 98, 0, unimplemented_handler },
    { 99, 0, unimplemented_handler },
    { 100, 0, unimplemented_handler },
    { 101, 0, unimplemented_handler },
    { 102, 0, unimplemented_handler },
    { 103, 0, unimplemented_handler },
    { 104, 0, unimplemented_handler },
    { 105, 0, unimplemented_handler },
    { 106, 0, unimplemented_handler },
    { 107, 0, unimplemented_handler },
    { 108, 0, unimplemented_handler },
    { 109, 0, unimplemented_handler },
    { 110, 0, unimplemented_handler },
    { 111, 0, unimplemented_handler },
    { 112, 0, unimplemented_handler },
    { 113, 0, unimplemented_handler },
    { 114, 0, unimplemented_handler },
    { 115, 0, unimplemented_handler },
    { 116, 0, unimplemented_handler },
    { 117, 0, unimplemented_handler },
    { 118, 0, unimplemented_handler },
    { 119, 0, unimplemented_handler },
    { 120, 0, unimplemented_handler },
    { 121, 0, unimplemented_handler },
    { 122, 0, unimplemented_handler },
    { 123, 0, unimplemented_handler },
    { 124, 0, unimplemented_handler },
    { 125, 0, unimplemented_handler },
    { 126, 0, unimplemented_handler },
    { 127, 0, unimplemented_handler },
    { 128, 0, unimplemented_handler },
    { 129, 0, unimplemented_handler },
    { 130, 0, unimplemented_handler },
    { 131, 0, unimplemented_handler },
    { 132, 0, unimplemented_handler },
    { 133, 0, unimplemented_handler },
    { 134, 0, unimplemented_handler },
    { 135, 0, unimplemented_handler },
    { 136, 0, unimplemented_handler },
    { 137, 0, unimplemented_handler },
    { 138, 0, unimplemented_handler },
    { 139, 0, unimplemented_handler },
    { 140, 0, unimplemented_handler },
    { 141, 0, unimplemented_handler },
    { 142, 0, unimplemented_handler },
    { 143, 0, unimplemented_handler },
    { 144, 0, unimplemented_handler },
    { 145, 0, unimplemented_handler },
    { 146, 0, unimplemented_handler },
    { 147, 0, unimplemented_handler },
    { 148, 0, unimplemented_handler },
    { 149, 0, unimplemented_handler },
    { 150, 0, unimplemented_handler },
    { 151, 0, unimplemented_handler },
    { 152, 0, unimplemented_handler },
    { 153, 0, unimplemented_handler },
    { 154, 0, unimplemented_handler },
    { 155, 0, unimplemented_handler },
    { 156, 0, unimplemented_handler },
    { 157, 0, unimplemented_handler },
    { 158, 0, unimplemented_handler },
    { 159, 0, unimplemented_handler },
    { 160, 0, unimplemented_handler },
    { 161, 0, unimplemented_handler },
    { 162, 0, unimplemented_handler },
    { 163, 0, unimplemented_handler },
    { 164, 0, unimplemented_handler },
    { 165, 0, unimplemented_handler },
    { 166, 0, unimplemented_handler },
    { 167, 0, unimplemented_handler },
    { 168, 0, unimplemented_handler },
    { 169, 0, unimplemented_handler },
    { 170, 0, unimplemented_handler },
    { 171, 0, unimplemented_handler },
    { 172, 0, unimplemented_handler },
    { 173, 0, unimplemented_handler },
    { 174, 0, unimplemented_handler },
    { 175, 0, unimplemented_handler },
    { 176, 0, unimplemented_handler },
    { 177, 0, unimplemented_handler },
    { 178, 0, unimplemented_handler },
    { 179, 0, unimplemented_handler },
    { 180, 0, unimplemented_handler },
    { 181, 0, unimplemented_handler },
    { 182, 0, unimplemented_handler },
    { 183, 0, unimplemented_handler },
    { 184, 0, unimplemented_handler },
    { 185, 0, unimplemented_handler },
    { 186, 0, unimplemented_handler },
    { 187, 0, unimplemented_handler },
    { 188, 0, unimplemented_handler },
    { 189, 0, unimplemented_handler },
    { 190, 0, unimplemented_handler },
    { 191, 0, unimplemented_handler },
    { 192, 0, unimplemented_handler },
    { 193, 0, unimplemented_handler },
    { 194, 0, unimplemented_handler },
    { 195, 0, unimplemented_handler },
    { 196, 0, unimplemented_handler },
    { 197, 0, unimplemented_handler },
    { 198, 0, unimplemented_handler },
    { 199, 0, unimplemented_handler },
    { 200, 0, unimplemented_handler },
    { 201, 0, unimplemented_handler },
    { 202, 0, unimplemented_handler },
    { 203, 0, unimplemented_handler },
    { 204, 0, unimplemented_handler },
    { 205, 0, unimplemented_handler },
    { 206, 0, unimplemented_handler },
    { 207, 0, unimplemented_handler },
    { 208, 0, unimplemented_handler },
    { 209, 0, unimplemented_handler },
    { 210, 0, unimplemented_handler },
    { 211, 0, unimplemented_handler },
    { 212, 0, unimplemented_handler },
    { 213, 0, unimplemented_handler },
    { 214, 0, unimplemented_handler },
    { 215, 0, unimplemented_handler },
    { 216, 0, unimplemented_handler },
    { 217, 0, unimplemented_handler },
    { 218, 0, unimplemented_handler },
    { 219, 0, unimplemented_handler },
    { 220, 0, unimplemented_handler },
    { 221, 0, unimplemented_handler },
    { 222, 0, unimplemented_handler },
    { 223, 0, unimplemented_handler },
    { 224, 0, unimplemented_handler },
    { 225, 0, unimplemented_handler },
    { 226, 0, unimplemented_handler },
    { 227, 0, unimplemented_handler },
    { 228, 0, unimplemented_handler },
    { 229, 0, unimplemented_handler },
    { 230, 0, unimplemented_handler },
    { 231, 0, unimplemented_handler },
    { 232, 0, unimplemented_handler },
    { 233, 0, unimplemented_handler },
    { 234, 0, unimplemented_handler },
    { 235, 0, unimplemented_handler },
    { 236, 0, unimplemented_handler },
    { 237, 0, unimplemented_handler },
    { 238, 0, unimplemented_handler },
    { 239, 0, unimplemented_handler },
    { 240, 0, unimplemented_handler },
    { 241, 0, unimplemented_handler },
    { 242, 0, unimplemented_handler },
    { 243, 0, unimplemented_handler },
    { 244, 0, unimplemented_handler },
    { 245, 0, unimplemented_handler },
    { 246, 0, unimplemented_handler },
    { 247, 0, unimplemented_handler },
    { 248, 0, unimplemented_handler },
    { 249, 0, unimplemented_handler },
    { 250, 0, unimplemented_handler },
    { 251, 0, unimplemented_handler },
    { 252, 0, unimplemented_handler },
    { 253, 0, unimplemented_handler },
    { 254, 0, unimplemented_handler },
    { 255, 0, unimplemented_handler }
);
