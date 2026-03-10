use std::{alloc::Layout, ptr::NonNull};

use crate::{loader::parser::function::Directive, memory::datumspace::{DatumAllocator, DatumspaceError}};

pub struct Runnable<'a>
{
    maxstack: usize,
    maxlocals: usize,
    directives: &'a [Directive],
    bytecode: &'a [u8],
}

impl<'a> Runnable<'a>
{
    /// Create a Runnable from raw data parsed by the loader's parser.
    ///
    /// This also checks the validity of that data. For example, if there
    /// isnt a maxstack or maxlocal directive specifying such data, then
    /// the runnable cannot be constructed.
    pub fn from_parsed_data<'file>(allocator: &mut DatumAllocator, directives: &'file [Directive], bytecode: &'file [u8]) -> Result<&'a Self, DatumspaceError>
    {
        const REQUIRED_DIRECTIVES_COUNT: usize = 2;
        let directive_count = directives.len() - REQUIRED_DIRECTIVES_COUNT;
        let directive_byte_count = size_of::<Directive>() * directive_count;

        // Allocate space for this new runnable
        let total_space = size_of::<Self>() + directive_byte_count + bytecode.len();
        let layout = Layout::array::<u8>(total_space).map_err(|_| DatumspaceError::AllocationFailure)?;

        let allocation = allocator.raw_alloc(layout).ok_or(DatumspaceError::AllocationFailure)?;

        let runnable: NonNull<Runnable> = allocation.cast();
        let directive_space: NonNull<Directive> = unsafe { allocation.byte_add(size_of::<Self>()) }.cast();
        let bytecode_space: NonNull<u8> = unsafe { directive_space.byte_add(directive_byte_count) }.cast();

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
