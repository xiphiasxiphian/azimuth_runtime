use std::{
    io,
    path::{Path, PathBuf},
};

use binrw::binread;

use crate::{
    loader::parser::{function::Directive},
    memory::{
        allocators::AllocatorError,
        datumspace::{Datumspace, DatumspaceError, datum::DatumPage, runnable::Runnable},
    },
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
pub struct SymbolId([u8; 16]);

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

    pub fn get_entrypoint(&'a mut self, entry_path: &str) -> Result<Option<&'a Runnable<'a>>, LoaderError>
    {
        let datum_page = self.get_page(entry_path)?;
        Ok(datum_page
            .functions
            .iter()
            .find(|x| x.directives().contains(&Directive::Start)))
    }

    pub fn get_function(&mut self, symbolic: &str) -> Result<&'a Runnable<'a>, LoaderError>
    {
        // Ensure the page is loaded
        let _page = self.get_page(symbolic)?;

        // Get the runnable out of datumspace
        self.datumspace.get_runnable(symbolic).map_err(LoaderError::DatumspaceError)
    }

    /// Gets a page references to by the symbolic path.
    /// If this page isn't currently loaded, it will try and find
    /// the file that corresponds to it, and loads it into datumspace
    ///
    fn get_page(&mut self, symbolic: &str) -> Result<DatumPage<'a>, LoaderError>
    {
        let (filepath, pagename) = self.extract_from_symbolic(symbolic);

        // Check if page is already loaded
        if let Ok(page) = self.datumspace.get_page(pagename) { return Ok(page) }

        let bytes = std::fs::read(filepath).map_err(|x| LoaderError::FileReadError(x))?;
        let layout = FileLayout::from_bytes(&bytes).ok_or(LoaderError::InvalidFileStructure)?;

        Ok(
            self
                .datumspace
                .load_datum(
                    pagename,
                    layout.constants(),
                    layout.functions(),
                )
                .map_err(|x| LoaderError::DatumspaceError(x))?
        )
    }

    fn extract_from_symbolic<'s>(&self, symbolic: &'s str) -> (PathBuf, &'s str)
    {
        /*
         * In general symbolic paths will take a couple different forms:
         *
         * path/to/module::symbol_name -> Relative to the execution base. These will
         * normally be source files as part of whatever is being ran
         *
         *
         *
         */

         // Strip off any possible the symbols name
         let symbolless = symbolic.rsplit_once("::").map_or(symbolic, |(x, _)| x);

         // For now, assume that they are all relative paths.
         // TODO: Work on internal symbols that require special treatment

         (self.base.join(symbolless), symbolless)
    }
}
