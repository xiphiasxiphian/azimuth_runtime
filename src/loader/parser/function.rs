use crate::{
    engine::opcodes::Opcode,
    guard,
    loader::parser::{
        bytes_to_numeric,
        table::{Table, TableEntry},
    },
    memory::datumspace::runnable::Runnable,
};

type DirectiveHandler = &'static dyn Fn(&[u8]) -> Option<Directive>; // Creates a handler

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Directive
{
    Symbol(u32, u32), // (name_index, descriptor_index)
    Start,
    MaxStack(u16),  // max_stack
    MaxLocals(u16), // max_locals
}

impl Directive
{
    const OPCODE: u8 = Opcode::Directive as u8; // Opcode for a directive
    const SYMBOL: u8 = 0; // The symbol directive is important and should always be 0

    const HEADER_SIZE: usize = 2; // Opcode (1 byte) + Directive Type (1 byte)

    const HANDLERS: [(usize, DirectiveHandler); 4] = [
        (8, &|x| {
            Some(Directive::Symbol(
                u32::from_le_bytes(x[0..4].try_into().ok()?),
                u32::from_le_bytes(x[4..8].try_into().ok()?),
            ))
        }),
        (0, &|_| Some(Directive::Start)),
        (2, &|x| Some(Directive::MaxStack(bytes_to_numeric!(u16, x)))),
        (2, &|x| Some(Directive::MaxLocals(bytes_to_numeric!(u16, x)))),
    ];
}

#[derive(Debug)]
pub struct FunctionInfo<'file>
{
    pub name_index: usize,
    pub directives: Vec<Directive>,
    pub code: &'file [u8],
}

impl<'file> FunctionInfo<'file>
{
    pub fn new(input: &'file [u8]) -> Option<(Self, &'file [u8])>
    {
        // Get symbol directive. The symbol directive
        // should be Directive 0, so get its entry in the handler array
        let &(symbol_operand_byte_count, symbol_handler) = Directive::HANDLERS.get(<usize>::from(Directive::SYMBOL))?;
        let (symbol_directive, rem_dirs) =
            input.split_at_checked(symbol_operand_byte_count + Directive::HEADER_SIZE)?;

        let symbol_operands = symbol_directive.get(Directive::HEADER_SIZE..)?;

        let (name_index, descriptor): (u32, u32) = symbol_handler(symbol_operands).and_then(|x| {
            match x
            {
                Directive::Symbol(name_index, code_count) => Some((name_index, code_count)),
                _ => None, // Something has gone really wrong if this triggers
            }
        })?;

        let mut directives: Vec<Directive> = vec![];
        let mut remaining = rem_dirs;

        // Loop through the bytes until it doesn't represent a directive anymore
        while let &[Directive::OPCODE, x, ref res @ ..] = remaining
        {
            // This means that there has been a second symbol directive which isnt
            // legal
            guard!(x != Directive::SYMBOL);

            // Parse the found directive
            let &(operand_count, handler) = Directive::HANDLERS.get(<usize>::from(x))?;
            let (operands, rem) = res.split_at_checked(operand_count)?;

            directives.push(handler(operands)?);

            remaining = rem;
        }

        #[expect(
            clippy::expect_used,
            reason = "Running this program on a less than 32-bit architecture isn't supported"
        )]
        {
            let (code_slice, remaining) = remaining.split_at_checked(
                descriptor
                    .try_into()
                    .expect("Running on a none 32-bit or 64-bit architecture. How? Why?"),
            )?;

            Some((
                Self {
                    name_index: <usize>::try_from(name_index)
                        .expect("Running on a none 32-bit or 64-bit architecture. How? Why?"),
                    directives,
                    code: code_slice,
                },
                remaining,
            ))
        }
    }

    pub fn get_all_functions(input: &'file [u8]) -> Option<(Vec<Self>, &'file [u8])>
    {
        let mut functions = vec![];
        let mut remaining = input;
        while let &[Directive::OPCODE, Directive::SYMBOL, ..] = remaining
        // There is another function to read
        {
            let (function, rem) = Self::new(remaining)?;
            functions.push(function);
            remaining = rem;
        }

        Some((functions, remaining))
    }

    pub fn has_directive(&self, directive: Directive) -> bool
    {
        self.directives.contains(&directive)
    }
}

#[cfg(test)]
mod function_info_tests
{
    use super::*;

    #[test]
    fn basic_function()
    {
        // Function with symbol directive and no other directives
        let data: [u8; 14] = [
            Directive::OPCODE,
            Directive::SYMBOL,
            0,
            0,
            0,
            0, // name index
            4,
            0,
            0,
            0, // code count
            // Code (4 bytes)
            0x01,
            0x02,
            0x03,
            0x04,
        ];
        let table = Table::new(vec![
            TableEntry::String("main"), // name index
            TableEntry::Integer(4),     // descriptor index
        ]);

        let (function, rem) = FunctionInfo::new(&data).expect("Failed to parse simple function");
        assert_eq!(function.directives.len(), 0); // Doesn't include symbol directive
        assert_eq!(function.code, vec![0x01, 0x02, 0x03, 0x04]);
        assert!(rem.is_empty());
    }
}
