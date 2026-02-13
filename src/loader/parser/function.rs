use crate::{engine::opcodes::Opcode, guard, loader::{parser::{Table, TableEntry, bytes_to_numeric}, runnable::Runnable}};

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
pub struct FunctionInfo
{
    directives: Vec<Directive>,

    // In the future this code section will be able to be a byte slice
    // (&[u8]) rather than an owned vector as the actual data will be stored in
    // metaspace somewhere.
    // However, as metaspace doesnt exist yet, right now it has to be
    // owned.
    code: Vec<u8>,
}

impl FunctionInfo
{
    pub fn new<'b>(input: &'b [u8], table: &Table) -> Option<(Self, &'b [u8])>
    {
        // Get symbol directive. The symbol directive
        // should be Directive 0, so get its entry in the handler array
        let &(symbol_operand_byte_count, symbol_handler) = Directive::HANDLERS.get(<usize>::from(Directive::SYMBOL))?;
        let (symbol_directive, rem_dirs) =
            input.split_at_checked(symbol_operand_byte_count + Directive::HEADER_SIZE)?;

        let symbol_operands = symbol_directive.get(Directive::HEADER_SIZE..)?;

        let (name, descriptor): (&str, u32) = symbol_handler(symbol_operands).and_then(|x| {
            match x
            {
                Directive::Symbol(name_index, code_count) =>
                {
                    // Even thought the name is not needed here, it is
                    // important still to verify that it is a valid constant pool entry,
                    // and does in fact refer to a string entry

                    // Get the name and descriptor from the constant pool.
                    // This will also check whether the given indices are in fact valid.
                    let name = table.get(name_index)?;

                    match *name
                    {
                        // The name should refer to a String, and the descriptor should refer to an Integer
                        TableEntry::String(ref name_str) => Some((name_str.as_str(), code_count)),
                        _ => None,
                    }
                }
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
        let (code_slice, remaining) = remaining.split_at_checked(
            descriptor
                .try_into()
                .expect("Running on a none 32-bit or 64-bit architecture. How? Why?"),
        )?;

        Some((
            Self {
                directives,
                code: code_slice.to_vec(),
            },
            remaining,
        ))
    }

    pub fn get_all_functions<'a>(input: &'a [u8], table: &Table) -> Option<(Vec<Self>, &'a [u8])>
    {
        let mut functions = vec![];
        let mut remaining = input;
        while let &[Directive::OPCODE, Directive::SYMBOL, ..] = remaining
        // There is another function to read
        {
            let (function, rem) = Self::new(remaining, table)?;
            functions.push(function);
            remaining = rem;
        }

        Some((functions, remaining))
    }

    /// Turn a raw parsed `FunctionInfo` into a usable `Runnable`, with safety checks
    pub fn into_runnable(&self) -> Option<Runnable<'_>>
    {
        Runnable::from_parsed_data(&self.directives, &self.code)
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
        let table = Table {
            entries: vec![
                TableEntry::String("main".into()), // name index
                TableEntry::Integer(4),            // descriptor index
            ],
        };

        let (function, rem) = FunctionInfo::new(&data, &table).expect("Failed to parse simple function");
        assert_eq!(function.directives.len(), 0); // Doesn't include symbol directive
        assert_eq!(function.code, vec![0x01, 0x02, 0x03, 0x04]);
        assert!(rem.is_empty());
    }
}
