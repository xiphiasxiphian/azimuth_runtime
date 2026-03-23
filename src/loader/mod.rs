use std::{fs::{File, read}, io, path::{Path, PathBuf}};

use crate::{loader::parser::FileLayout, memory::{allocators::AllocatorError, datumspace::{Datumspace, runnable::Runnable}}};

pub(super) mod parser;

const DEFAULT_CAPACITY: usize = 1 << 24; // 16 MiB

pub struct Loader<'a>
{
    datumspace: Datumspace<'a>,
    base: &'a Path,
}

#[derive(Debug)]
pub enum LoaderError
{
    FileReadError(io::Error),
    AllocatorError(AllocatorError),
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl<'a> Loader<'a>
{
    pub fn new(base: &'a str) -> Result<Self, LoaderError>
    {
        Ok(
            Self {
                datumspace: Datumspace::with_capacity(DEFAULT_CAPACITY).map_err(|x| LoaderError::AllocatorError(x))?,
                base: Path::new(base),
            }
        )
    }

    pub fn get_entrypoint(&mut self, filename: &str) -> Result<(), LoaderError>
    {
        let bytes = std::fs::read(self.base.join(filename)).map_err(|x| LoaderError::FileReadError(x))?;
        let layout = FileLayout::from_bytes(&bytes);
    }

    pub fn get_function(path: &str) -> Result<&'a Runnable<'a>, LoaderError>
    {
        todo!()
    }
}
