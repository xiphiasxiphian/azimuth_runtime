use std::{fs::File, path::Path};

use binrw::{BinRead, io::BufReader};

use crate::loader::{LoaderError, parser::layout::FileLayout};

pub mod function;
pub mod table;
pub mod layout;

pub fn parse_file(filepath: &Path) -> Result<FileLayout, LoaderError>
{
    let file = File::open(filepath).map_err(LoaderError::FileReadError)?;
    let mut reader = BufReader::new(file);

    FileLayout::read(&mut reader).map_err(LoaderError::InvalidFileStructure)
}

#[cfg(test)]
mod parser_tests
{}
