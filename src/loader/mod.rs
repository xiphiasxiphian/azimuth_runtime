use std::{fs::read, io};

use crate::loader::{
    constant_table::ConstantTable,
    parser::{Directive, FileLayout, FunctionInfo},
    runnable::Runnable,
};

pub mod constant_table;
pub(super) mod parser;
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
        let file_contents = read(filename).map_err(LoaderError::FileReadError)?;
        let layout = FileLayout::from_bytes(&file_contents).ok_or(LoaderError::LayoutError)?;

        Ok(Self { layout })
    }

    // Get the entry point (aka function marked with .start)
    pub fn get_entry_point(&self) -> Option<Runnable>
    {
        println!("{:?}", self.layout.functions());
        self.layout
            .functions()
            .iter()
            .find(|x| x.has_directive(Directive::Start))
            .and_then(FunctionInfo::into_runnable)
    }

    pub fn get_constant_table(&self) -> ConstantTable
    {
        ConstantTable::from_parsed_table(self.layout.constants())
    }
}
