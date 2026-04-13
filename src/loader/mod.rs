use std::{
    io,
    path::Path, ptr::NonNull,
};

use binrw::binread;

use crate::{loader::parser::parse_file, memory::{
        allocators::AllocatorError,
        datumspace::{Datumspace, DatumspaceError, datum::DatumPage, runnable::{Function, FunctionFlags, Runnable}, tables::symbol_table::Symbol}, stack::entry,
    }};

pub(super) mod parser;

/// 128-bit content-derived identifier for a symbol.
///
/// For functions and types, computed as: hash(namespace + name + type_descriptor), truncated to 16 bytes.
/// Being content-derived means two independent compilers targeting the same
/// source produce identical UUIDs, enabling cross-compiler linking.
///
/// For modules, Computed from the file's fully-qualified module path + version.
/// Used by the import table to verify the correct file was loaded.
#[binread]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SymbolId(pub [u8; 16]);

const DEFAULT_CAPACITY: usize = 1 << 24; // 16 MiB

pub struct Loader<'a>
{
    datumspace: Datumspace<'a>,
    base: SymbolId,
}

#[derive(Debug)]
pub enum LoaderError
{
    FileReadError(io::Error),
    InvalidFileStructure(binrw::Error),
    FailedToFindSymbol,
    AllocatorError(AllocatorError),
    DatumspaceError(DatumspaceError),
    MissingEntrypoint,
}

impl<'a> Loader<'a>
{
    pub fn new(base: &'a str) -> Result<Self, LoaderError>
    {
        let mut datumspace = Datumspace::with_capacity(DEFAULT_CAPACITY).map_err(|x| LoaderError::AllocatorError(x))?;

        // Load initial page
        let parsed_file = parse_file(Path::new(base))?;
        let initial_page = datumspace.load_datum(&parsed_file).map_err(LoaderError::DatumspaceError)?;

        Ok(
            Self {
                datumspace,
                base: *initial_page.id
            }
        )
    }

    /*
     * TODO:
     * Rework all the required loader functions for the new datumspace setup
     *
     * This mainly includes:
     * - Getting functions and entrypoint
     * - Ensuring pages are loaded when information from them is required
     * - Parsing new files when required and calling `load_datum`
     *
     */

     pub fn get_entrypoint(&mut self) -> Result<FunctionInfo<'a>, LoaderError>
     {
         /*
          * - Load the initial page
          * - Find the entrypoint within that page
          * - Resolve the code location
          * - Return in wrapped format
          */

          // Ensure the page is loaded
          let page = self.datumspace.get_page(&self.base).map_err(LoaderError::DatumspaceError)?;
          let entrypoint = page.functions
              .iter()
              .find_map(|x| match x {
                  Runnable::Function(f) if f.flags == FunctionFlags::ENTRYPOINT => Some(f),
                  _ => None,
              })
              .ok_or(LoaderError::MissingEntrypoint)?;

          let (maxstack, maxlocals) = entrypoint.setup_info();
          let bytecode = self.datumspace.resolve_location(&self.base, entrypoint.bytecode)
            .map_err(LoaderError::DatumspaceError)?;

          Ok(
              FunctionInfo { maxstack, maxlocals, bytecode }
          )
     }
}

// Wrapper Structs


pub struct FunctionInfo<'a>
{
    pub maxstack: usize,
    pub maxlocals: usize,
    pub bytecode: &'a [u8]
}
