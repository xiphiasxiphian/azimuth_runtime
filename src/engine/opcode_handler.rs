use std::array::from_fn;

struct HandlerInputInfo
{
    opcode: u8,
    params: Vec<u8>,
}

#[derive(Clone, Copy)]
struct HandlerInfo
{
    opcode: u8,
    param_count: u8,
    handler: fn(HandlerInputInfo),
}

const HANDLERS: [HandlerInfo; 256] = [HandlerInfo {opcode: 0, param_count: 0, handler: unimplemented_handler}; 256];

fn unimplemented_handler(input: HandlerInputInfo)
{
    panic!("Opcode not implemented");
}
