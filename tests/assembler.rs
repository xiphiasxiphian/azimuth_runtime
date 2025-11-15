use std::{collections::HashMap, error::Error, fmt::Display, io::Write, sync::LazyLock};

#[derive(Debug, Clone, Copy)]
enum OperandType
{
    Int,
    WideInt,
}

static OPCODES: LazyLock<HashMap<&'static str, (u8, &'static [OperandType])>> =
    LazyLock::new(|| {
        HashMap::from([
            ("nop", (0, [].as_slice())),
            ("i4.const.0", (1, [].as_slice())),
            ("i4.const.1", (2, [].as_slice())),
            ("i4.const.2", (3, [].as_slice())),
            ("i4.const.3", (4, [].as_slice())),
            ("i8.const.0", (5, [].as_slice())),
            ("i8.const.1", (6, [].as_slice())),
            ("i8.const.2", (7, [].as_slice())),
            ("i8.const.3", (8, [].as_slice())),
        ])
    });

static DIRECTIVES: LazyLock<HashMap<&'static str, (u8, &'static [OperandType])>> =
    LazyLock::new(|| {
        HashMap::from([
            (".start", (0, [].as_slice())),
            (".symbol", (1, [OperandType::WideInt].as_slice())),
            (".maxstack", (2, [OperandType::WideInt].as_slice())),
            (".maxlocal", (3, [OperandType::WideInt].as_slice())),
        ])
    });

#[derive(Debug, Clone, Copy)]
pub enum AssemblerError
{
    BadFormat,
    UnknownOpcode,
    UnknownDirective,
    WriteError,
    IncorrectOperandCount,
    OperandParseError(OperandType),
}

impl Display for AssemblerError
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
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

fn assemble_instruction<'a>(
    operation: &mut impl Iterator<Item = &'a str>,
    target: &mut dyn Write,
) -> AssemblerResult<()>
{
    const MAX_BYTES: usize = 4;

    let mut bytes: [u8; 4] = [0; 4];
    let (operand_types, written) = get_opcode_data(operation, &mut bytes)?;

    let mut byte_pointer: usize = written;
    for (operand, operand_type) in operation.zip(operand_types)
    {
        assert!(byte_pointer < MAX_BYTES);
        byte_pointer += parse_operand(operand, *operand_type, &mut bytes[byte_pointer..])?;
    }

    target
        .write_all(&bytes[..byte_pointer])
        .map_err(|_| AssemblerError::WriteError)?;
    Ok(())
}

fn get_opcode_data<'a>(
    operation: &mut impl Iterator<Item = &'a str>,
    bytes: &mut [u8],
) -> AssemblerResult<(&'a [OperandType], usize)>
{
    const DIRECTIVE_CODE: u8 = 254;

    let opcode = operation.next().ok_or(AssemblerError::BadFormat)?;
    if opcode.starts_with('.')
    {
        DIRECTIVES
            .get(opcode)
            .map(|(x, y)| {
                bytes[0..2].copy_from_slice(&[DIRECTIVE_CODE, *x]);
                (*y, 2)
            })
            .ok_or(AssemblerError::UnknownDirective)
    }
    else
    {
        OPCODES
            .get(opcode)
            .map(|(x, y)| {
                bytes[0] = *x;
                (*y, 1)
            })
            .ok_or(AssemblerError::UnknownOpcode)
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
        _ => Err(AssemblerError::UnknownDirective),
    }
}

fn parse_operand(
    operand: &str,
    operand_type: OperandType,
    bytes: &mut [u8],
) -> AssemblerResult<usize>
{
    return Ok(match operand_type
    {
        OperandType::Int =>
        {
            let byte: u8 = operand
                .parse::<u8>()
                .map_err(|_| AssemblerError::OperandParseError(operand_type))?;
            bytes[0] = byte;
            1
        }
        OperandType::WideInt =>
        {
            let number: u16 = operand
                .parse::<u16>()
                .map_err(|_| AssemblerError::OperandParseError(operand_type))?;
            bytes[0..].copy_from_slice(&number.to_le_bytes());
            2
        }
    });
}
