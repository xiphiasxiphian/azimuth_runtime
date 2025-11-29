use std::{fs::read, io};

use crate::loader::{parser::FileLayout, runnable::Runnable};

mod parser;
pub mod runnable;

pub struct Loader
{
    layout: FileLayout,
}

#[derive(Debug)]
enum LoaderError
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

    pub fn get_entry_point(&self) -> Option<Runnable>
    {
        todo!()
    }
}
