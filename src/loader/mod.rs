use std::{
    io,
    path::Path, ptr::NonNull,
};

use binrw::binread;

use crate::memory::{
        allocators::AllocatorError,
        datumspace::{Datumspace, DatumspaceError, datum::DatumPage, runnable::{Function, Runnable}, tables::symbol_table::Symbol},
    };

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
    base: &'a Path,
}

#[derive(Debug)]
pub enum LoaderError
{
    FileReadError(io::Error),
    InvalidFileStructure(binrw::Error),
    FailedToFindSymbol,
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

     pub fn get_entrypoint(&self) -> Result<FunctionInfo, LoaderError>
     {
         todo!()
     }
}

// Wrapper Structs


pub struct FunctionInfo<'a>
{
    maxstack: usize,
    maxlocals: usize,
    bytecode: &'a mut [u8]
}

impl<'a> FunctionInfo<'a>
{
    pub fn from_datumspace(function: &Function, module_id: SymbolId) -> Self
    {
        // These values should already have been verified
        let (maxstack, maxlocals) = <usize>::try_from(function.maxstack)
            .and_then(|x| {
                <usize>::try_from(function.maxlocals)
                    .map(|y| (x, y))
            })
            .expect("Invalid setup information not filtered out in loading");


    }
}
