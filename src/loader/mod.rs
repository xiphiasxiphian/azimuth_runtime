use crate::engine::opcodes::Opcode;

pub enum Directive
{
    Start,
    MaxStack(u16),
    MaxLocals(u16),
}

pub struct Loader<'a>
{
    runnables: Vec<Runnable<'a>>,
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl<'a> Loader<'a>
{
    const DIRECTIVE_OPCODE: u8 = Opcode::Directive as u8;

    pub fn from_file(filename: &str) -> Option<Self>
    {
        let contents = std::fs::read(filename).ok()?;

        // Read off directives for function metadata

    }

    fn match_off(input: &'a [u8]) -> Result<(Directive, &'a [u8]), Option<&'a [u8]>>
    {
        match input
        {
            [Self::DIRECTIVE_OPCODE, 0, rem @ ..] => Ok((Directive::Start, rem)),
            [Self::DIRECTIVE_OPCODE, 1, b1, b2, rem @ ..] => Ok((Directive::MaxStack(u16::from_le_bytes([*b1, *b2])), rem)),
            [Self::DIRECTIVE_OPCODE, 2, b1, b2, rem @ ..] => Ok((Directive::MaxLocals(u16::from_le_bytes([*b1, *b2])), rem)),
            code => Err(Some(code))
        }
    }
}

pub struct Runnable<'a>
{
    maxstack: usize,
    maxlocals: usize,
    directives: Vec<Directive>,
    bytecode: &'a [u8]
}

impl<'a> Runnable<'a>
{
    pub fn new() -> Self
    {

    }
}
