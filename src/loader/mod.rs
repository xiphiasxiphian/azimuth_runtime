use crate::engine::opcodes::Opcode;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Directive
{
    Start,
    MaxStack(u16),
    MaxLocals(u16),
}

pub struct Loader
{
    runnables: Vec<Runnable>,
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl Loader
{
    pub fn from_file(filename: &str) -> Option<Self>
    {
        // Again there is definitely a better way of doing this that doesn't
        // involve spamming mutable variables everywhere
        // But this entire system is going to get rewritten as some point
        // and I just want it working right now.

        let contents = std::fs::read(filename).ok()?;
        let mut runnables: Vec<Runnable> = vec![];

        let mut remaining: &[u8] = &contents;
        while let [_, ..] = remaining
        {
            let (runnable, rem) = Runnable::from_bytes(remaining)?;
            runnables.push(runnable);
            remaining = rem;
        }


        Some(Self {
            runnables
        })
    }

    pub fn get_entry_point(&self) -> Option<&Runnable>
    {
        self.runnables.iter().find(|x| x.directives.contains(&Directive::Start))
    }
}

pub struct Runnable
{
    maxstack: usize,
    maxlocals: usize,
    directives: Vec<Directive>,
    bytecode: Vec<u8>,
}

impl Runnable
{
    const DIRECTIVE_OPCODE: u8 = Opcode::Directive as u8;

    pub fn from_bytes(input: &[u8]) -> Option<(Self, &[u8])>
    {
        // Read off directives for function metadata
        let mut directives: Vec<Directive> = vec![];

        // There might be a way of doing this with less mutability
        let mut remaining = input;
        loop
        {
            match Self::match_off(remaining)
            {
                Ok((dir, rem)) => {
                    directives.push(dir);
                    remaining = rem;
                }
                Err(Some(code)) => {
                    let bytecode = Vec::from_iter(code.iter().copied());
                    let rem = &remaining[bytecode.len()..];
                    return Some(
                        (
                            Self::from_parsed_data(directives, bytecode)?,
                            rem
                        )
                    )
                }
                Err(None) => {
                    return None;
                }
            }
        }
    }

    fn from_parsed_data(directives: Vec<Directive>, bytecode: Vec<u8>) -> Option<Self>
    {
        // This right now is fairly shit but I just want it working
        // The entire loader is going to be changed at some point down the line anyway
        let mut max_stack: Option<usize> = None;
        let mut max_locals: Option<usize> = None;

        // This is all just soo bad
        let (required, directives) = directives.iter().partition(|x| matches!(x, Directive::MaxStack(_) | Directive::MaxLocals(_)));

        for directive in required
        {
            match directive
            {
                Directive::MaxStack(x) => {
                    max_stack.replace(x.into()).map_or(Some(()), |_| None)?;
                }
                Directive::MaxLocals(x) => {
                    max_locals.replace(x.into()).map_or(Some(()), |_| None)?;
                }
                _ => unreachable!()
            }
        }

        Some(Self {
            maxstack: max_stack?,
            maxlocals: max_locals?,
            directives,
            bytecode: bytecode,
        })
    }

    fn match_off(input: &[u8]) -> Result<(Directive, &[u8]), Option<&[u8]>>
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
