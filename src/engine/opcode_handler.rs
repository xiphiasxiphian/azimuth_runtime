#[derive(Clone, Copy)]
struct HandlerInfo
{
    opcode: u8,
    param_count: u8,
    handler: fn(),
}

const HANDLERS: [HandlerInfo; 256] = [HandlerInfo {opcode: 0, param_count: 0, handler: unimplemented_handler}; 256];

fn unimplemented_handler()
{
    panic!("Opcode not implemented");
}
