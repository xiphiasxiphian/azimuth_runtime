use std::{fs::read, io};

use crate::loader::{parser::{Directive, FileLayout}, runnable::Runnable};

mod parser;
pub mod runnable;

pub struct Loader
{
    layout: FileLayout,
}

#[derive(Debug)]
pub enum LoaderError
{
    FileReadError(io::Error),
    LayoutError,
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl Loader
{
    pub fn from_file(filename: &str) -> Result<Self, LoaderError>
    {
        let file_contents = read(filename).map_err(|x| LoaderError::FileReadError(x))?;
        let layout = FileLayout::from_bytes(&file_contents).ok_or(LoaderError::LayoutError)?;

        Ok(Self { layout })
    }

    // Get the entry point (aka function marked with .start)
    pub fn get_entry_point(&self) -> Option<Runnable>
    {
        self.layout.functions().iter()
            .find(|x| x.has_directive(Directive::Start))
            .and_then(|x| x.into_runnable())
    }
}
