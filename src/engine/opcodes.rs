#[derive(Clone, Copy)]
pub enum Opcode
{
    Nop,
    IConst0,
    IConst1,
    IConst2,
    IConst3,
    F4Const0,
    F4Const1,
    F8Const0,
    F8Const1,
    IConst,
    IConstW,
    Const,
    Directive = 254,
    Unimplemented = 255,
}
