use std::{
    fs::{File, read},
    io,
    path::{Path, PathBuf},
};

use crate::{
    loader::parser::{FileLayout, function::Directive},
    memory::{
        allocators::AllocatorError,
        datumspace::{Datumspace, DatumspaceError, runnable::Runnable},
    },
};

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
    InvalidFilepathEncoding,
    InvalidFileStructure,
    AllocatorError(AllocatorError),
    DatumspaceError(DatumspaceError),
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl<'a> Loader<'a>
{
    pub fn new(base: &'a str) -> Result<Self, LoaderError>
    {
        Ok(Self {
            datumspace: Datumspace::with_capacity(DEFAULT_CAPACITY).map_err(|x| LoaderError::AllocatorError(x))?,
            base: Path::new(base),
        })
    }

    pub fn get_entrypoint(&'a mut self, filename: &str) -> Result<Option<&Runnable<'a>>, LoaderError>
    {
        let bytes = std::fs::read(self.base.join(filename)).map_err(|x| LoaderError::FileReadError(x))?;
        let layout = FileLayout::from_bytes(&bytes).ok_or(LoaderError::InvalidFileStructure)?;

        let datum_page = self
            .datumspace
            .load_datum(
                self.base
                    .join(filename)
                    .to_str()
                    .ok_or(LoaderError::InvalidFilepathEncoding)?,
                layout.constants(),
                layout.functions(),
            )
            .map_err(|x| LoaderError::DatumspaceError(x))?;

        Ok(datum_page
            .functions
            .iter()
            .find(|x| x.directives().contains(&Directive::Start)))
    }

    pub fn get_function(path: &str) -> Result<&'a Runnable<'a>, LoaderError>
    {
        todo!()
    }
}
