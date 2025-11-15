use std::{error::Error, fmt::Display, io::Write, iter::repeat_n};

#[derive(Debug)]
pub enum AssemblerError
{
    BadFormat,
    OpcodeNotFound,
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
        assemble_instruction(line.split_whitespace(), target)?;
    }
    Ok(())
}

fn assemble_instruction<'a>(mut operation: impl Iterator<Item = &'a str>, target: &mut dyn Write) -> AssemblerResult<()>
{
    let opcode = operation.next().ok_or(AssemblerError::BadFormat)?;
    let (opcode_byte, param_count) = get_opcode_data(opcode)?;

    let operands: Vec<u8> = [opcode_byte].into_iter().chain(operation.map(parse_operand).flatten()).collect();
    if operands.len() - 1 > param_count { return Err(AssemblerError::IncorrectOperandCount) }

    target.write_all(&operands).map_err(|_| AssemblerError::WriteError)?;
    Ok(())
}

fn get_opcode_data(opcode: &str) -> AssemblerResult<(u8, usize)>
{
    match opcode {
        "nop" => Ok((0, 0)),
        "i4.const.0" => Ok((1, 0)),
        _ => Err(AssemblerError::OpcodeNotFound)
    }
}

fn parse_operand(operand: &str) -> AssemblerResult<u8>
{
    todo!()
}
