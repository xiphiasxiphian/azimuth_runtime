use std::{alloc::Layout, ptr::NonNull};

use crate::{loader::parser::function::Directive, memory::datumspace::{DatumAllocator, DatumspaceError}};

#[derive(PartialEq, Eq, Debug)]
pub struct Runnable<'a>
{
    pub(super) name: &'a str,
    pub(super) maxstack: usize,
    pub(super) maxlocals: usize,
    pub(super) directives: &'a [Directive],
    pub(super) bytecode: &'a [u8],
}

impl<'a> Runnable<'a>
{
    /// Create a Runnable from raw data parsed by the loader's parser.
    ///
    /// This also checks the validity of that data. For example, if there
    /// isnt a maxstack or maxlocal directive specifying such data, then
    /// the runnable cannot be constructed.
    pub unsafe fn from_parsed_data<'file>(
        dst: NonNull<Runnable<'a>>,
        directive_space: NonNull<Directive>,
        bytecode_space: NonNull<u8>,
        name: &'a str,
        directives: &'file [Directive],
        bytecode: &'file [u8]
    ) -> Result<&'a Self, DatumspaceError>
    {
        const REQUIRED_DIRECTIVES_COUNT: usize = 2;
        let directive_count = directives.len() - REQUIRED_DIRECTIVES_COUNT;

        let runnable: NonNull<Runnable> = dst.cast();

        // Write bytecode in
        unsafe { bytecode_space.copy_from_nonoverlapping(NonNull::new_unchecked(bytecode.as_ptr() as *mut _), bytecode.len()) };

        directives
            .iter()
            .try_fold(
                // Collect the required data, checking for invalid states
                (None, None, 0),
                |(max_stack, max_locals, count), directive| match (max_stack, max_locals, *directive)
                {
                    (Some(_), _, Directive::MaxStack(_)) | (_, Some(_), Directive::MaxLocals(_)) => None,
                    (None, ml, Directive::MaxStack(x)) => Some((Some(x.into()), ml, count)),
                    (ms, None, Directive::MaxLocals(x)) => Some((ms, Some(x.into()), count)),
                    (ms, ml, optional) =>
                    {
                        if count >= directive_count { return None }

                        unsafe { directive_space.as_ptr().add(count).write(optional) };
                        Some((ms, ml, count + 1))
                    }
                },
            )
            .and_then(|(max_stack, max_locals, count)| {
                assert_eq!(count, directive_count);

                unsafe {
                    // Construct the runnable based on this data
                    runnable.write(Self {
                        name,
                        maxstack: max_stack?,
                        maxlocals: max_locals?,
                        directives: std::slice::from_raw_parts(directive_space.as_ptr(), count),
                        bytecode: std::slice::from_raw_parts(bytecode_space.as_ptr(), bytecode.len()),
                    });

                    Some(runnable.as_ref())
                }
            })
            .ok_or(DatumspaceError::InvalidStructure)
    }

    pub fn directives(&self) -> &[Directive]
    {
        &self.directives
    }

    /// Returns information critical to the setup of an executing process.
    ///
    /// This is mainly the max stack and the max locals space.
    pub fn setup_info(&self) -> (usize, usize)
    {
        (self.maxstack, self.maxlocals)
    }

    pub fn code(&self) -> &[u8]
    {
        self.bytecode
    }
}
