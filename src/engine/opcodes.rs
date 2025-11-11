#[derive(Clone, Copy)]
pub enum Opcode
{
    Nop,
    I4Const0,
    I4Const1,
    I4Const2,
    I4Const3,
    I8Const0,
    I8Const1,
    I8Const2,
    I8Const3,
    F4Const0,
    F4Const1,
    F8Const0,
    F8Const1,
    Unimplemented = 255
}
