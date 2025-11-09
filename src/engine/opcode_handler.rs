#[derive(Debug, Clone, Copy)]
struct HandlerInputInfo<'a>
{
    opcode: u8,
    params: &'a [u8],
}

#[derive(Clone, Copy)]
struct HandlerInfo<'a>
{
    opcode: u8,
    param_count: u8,
    handler: &'a dyn Fn(HandlerInputInfo) -> Option<usize>,
}

const HANDLERS: [HandlerInfo; 256] = [HandlerInfo { opcode: 0, param_count: 0, handler: &unimplemented_handler }; 256];

pub fn exec_instruction(bytecode: &[u8], pc: usize) -> usize
{
    let opcode = bytecode[pc];
    let handler_info = &HANDLERS[opcode as usize];

    assert!(opcode == handler_info.opcode, "HANDLERS Array invalid: misaligned opcode");
    let new_pc = pc + handler_info.param_count as usize + 1;

    (handler_info.handler)(HandlerInputInfo { opcode, params: &bytecode[(pc + 1)..new_pc] }).unwrap_or(new_pc)
}

fn debug_handler(input: HandlerInputInfo) -> Option<usize>
{
    dbg!(input);
    None
}

fn unimplemented_handler(input: HandlerInputInfo) -> Option<usize>
{
    panic!("Opcode not implemented")
}
