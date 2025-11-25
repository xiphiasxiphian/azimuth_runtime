use crate::loader::parser::Directive;

pub struct Runnable
{
    maxstack: usize,
    maxlocals: usize,
    directives: Vec<Directive>,
    bytecode: Vec<u8>,
}

impl Runnable
{

    pub fn from_parsed_data(directives: &[Directive], bytecode: Vec<u8>) -> Option<Self>
    {
        directives.iter()
            .try_fold((None, None, vec![]), |(max_stack, max_locals, mut optionals), directive| {
                match (max_stack, max_locals, *directive)
                {
                    (Some(_), _, Directive::MaxStack(_)) | (_, Some(_), Directive::MaxLocals(_)) => None,
                    (None, ml, Directive::MaxStack(x)) =>
                    {
                        Some((Some(x.into()), ml, optionals))
                    }
                    (ms, None, Directive::MaxLocals(x)) =>
                    {
                        Some((ms, Some(x.into()), optionals))
                    }
                    (ms, ml, x) => {
                        optionals.push(x);
                        Some((ms, ml, optionals))
                    },

                }
            }
        )
        .and_then(|(max_stack, max_locals, optionals)| Some(Self {
            maxstack: max_stack?,
            maxlocals: max_locals?,
            directives: optionals,
            bytecode,
        }))
    }

    pub fn directives(&self) -> &[Directive]
    {
        &self.directives
    }

    pub fn setup_info(&self) -> (usize, usize)
    {
        (self.maxstack, self.maxlocals)
    }

    // fn match_off(input: &[u8]) -> Result<(Directive, &[u8]), Option<&[u8]>>
    // {
    //     match input
    //     {
    //         [Self::DIRECTIVE_OPCODE, 0, rem @ ..] => Ok((Directive::Start, rem)),
    //         [Self::DIRECTIVE_OPCODE, 1, b1, b2, rem @ ..] => Ok((Directive::MaxStack(u16::from_le_bytes([*b1, *b2])), rem)),
    //         [Self::DIRECTIVE_OPCODE, 2, b1, b2, rem @ ..] => Ok((Directive::MaxLocals(u16::from_le_bytes([*b1, *b2])), rem)),
    //         code => Err(Some(code))
    //     }
    // }

    // pub fn from_bytes(input: &[u8]) -> Option<(Self, &[u8])>
    // {
    //     // Read off directives for function metadata
    //     let mut directives: Vec<Directive> = vec![];

    //     // There might be a way of doing this with less mutability
    //     let mut remaining = input;
    //     loop
    //     {
    //         match Self::match_off(remaining)
    //         {
    //             Ok((dir, rem)) => {
    //                 directives.push(dir);
    //                 remaining = rem;
    //             }
    //             Err(Some(code)) => {
    //                 let bytecode = Vec::from_iter(code.iter().copied());
    //                 let rem = &remaining[bytecode.len()..];
    //                 return Some(
    //                     (
    //                         Self::from_parsed_data(directives, bytecode)?,
    //                         rem
    //                     )
    //                 )
    //             }
    //             Err(None) => {
    //                 return None;
    //             }
    //         }
    //     }
    // }
}
