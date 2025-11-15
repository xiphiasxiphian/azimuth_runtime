use std::{collections::HashMap, error::Error, fmt::Display, io::Write, sync::LazyLock};

static OPCODES: LazyLock<HashMap<&str, (u8, usize)>> = LazyLock::new(|| {
    HashMap::from([
        ("nop", (0, 0)),
        ("i4.const.0", (1, 0))
    ])
});

static DIRECTIVES: LazyLock<HashMap<&str, (u8, usize)>> = LazyLock::new(|| {
    HashMap::from([
        (".start", (0, 0)),
        (".symbol", (1, 0)),
        (".maxstack", (2, 0)),
        (".maxlocal", (3, 0)),
    ])
});

#[derive(Debug)]
pub enum AssemblerError
{
    BadFormat,
    UnknownOpcode,
    UnknownDirective,
    WriteError,
    IncorrectOperandCount,
}

impl Display for AssemblerError
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for AssemblerError {}

type AssemblerResult<T> = Result<T, AssemblerError>;

pub fn assemble(input: &str, target: &mut dyn Write) -> AssemblerResult<()>
{
    for line in input.split('\n').filter(|x| *x != "")
    {
        assemble_instruction(&mut line.split_whitespace(), target)?;
    }
    Ok(())
}

fn assemble_instruction<'a>(operation: &mut impl Iterator<Item = &'a str>, target: &mut dyn Write) -> AssemblerResult<()>
{
    const MAX_BYTES: usize = 4;

    let mut bytes: [u8; 4] = [0; 4];
    let (param_count, written) = get_opcode_data(operation, &mut bytes)?;

    let mut byte_pointer: usize = written;
    for (i, operand) in operation.enumerate()
    {
        assert!(byte_pointer < MAX_BYTES);
        if i >= param_count { return Err(AssemblerError::IncorrectOperandCount) }

        byte_pointer += parse_operand(operand, &mut bytes[byte_pointer..])?;
    }

    target.write_all(&bytes).map_err(|_| AssemblerError::WriteError)?;
    Ok(())
}

fn get_opcode_data<'a>(operation: &mut impl Iterator<Item = &'a str>, bytes: &mut [u8]) -> AssemblerResult<(usize, usize)>
{
    const DIRECTIVE_CODE: u8 = 254;

    let opcode = operation.next().ok_or(AssemblerError::BadFormat)?;
    if opcode.starts_with('.')
    {
        DIRECTIVES.get(opcode).map(|(x, y)| {
            bytes[0..2].copy_from_slice(&[DIRECTIVE_CODE, *x]);
            (*y, 2)
        }).ok_or(AssemblerError::UnknownDirective)
    }
    else
    {
        OPCODES.get(opcode).map(|(x, y)| {
            bytes[0] = *x;
            (*y, 1)
        }).ok_or(AssemblerError::UnknownOpcode)
    }
}

fn parse_directive(directive: &str) -> AssemblerResult<(u8, usize)>
{
    match directive
    {
        ".start" => Ok((0, 0)),
        ".symbol" => Ok((1, 1)),
        ".maxstack" => Ok((2, 1)),
        ".maxlocal" => Ok((3, 1)),
        _ => Err(AssemblerError::UnknownDirective)
    }
}

fn parse_operand(operand: &str, bytes: &mut [u8]) -> AssemblerResult<usize>
{
    todo!()
}
