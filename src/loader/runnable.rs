use crate::loader::parser::Directive;

pub struct Runnable<'a>
{
    maxstack: usize,
    maxlocals: usize,
    directives: Vec<Directive>,
    bytecode: &'a [u8],
}

impl<'a> Runnable<'a>
{
    pub fn from_parsed_data(directives: &[Directive], bytecode: &'a [u8]) -> Option<Self>
    {
        directives
            .iter()
            .try_fold(
                (None, None, vec![]),
                |(max_stack, max_locals, mut optionals), directive| match (max_stack, max_locals, *directive)
                {
                    (Some(_), _, Directive::MaxStack(_)) | (_, Some(_), Directive::MaxLocals(_)) => None,
                    (None, ml, Directive::MaxStack(x)) => Some((Some(x.into()), ml, optionals)),
                    (ms, None, Directive::MaxLocals(x)) => Some((ms, Some(x.into()), optionals)),
                    (ms, ml, x) =>
                    {
                        optionals.push(x);
                        Some((ms, ml, optionals))
                    }
                },
            )
            .and_then(|(max_stack, max_locals, optionals)| {
                Some(Self {
                    maxstack: max_stack?,
                    maxlocals: max_locals?,
                    directives: optionals,
                    bytecode,
                })
            })
    }

    pub fn directives(&self) -> &[Directive]
    {
        &self.directives
    }

    pub fn setup_info(&self) -> (usize, usize)
    {
        (self.maxstack, self.maxlocals)
    }

    pub fn code(&self) -> &[u8]
    {
        self.bytecode
    }
}
