use std::{
    io, mem::transmute, path::Path, ptr::NonNull
};

use binrw::binread;
use itertools::Itertools;

use crate::{loader::parser::parse_file, memory::{
        allocators::AllocatorError,
        datumspace::{Datumspace, DatumspaceError, datum::DatumPage, runnable::{Function, FunctionFlags, Runnable}, tables::{constant_table::{Constant, ConstantTableEntry}, link_table::Link, symbol_table::Symbol}}, stack::entry,
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

#[derive(Debug)]
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
    InvalidLink,
}

impl From<DatumspaceError> for LoaderError
{
    fn from(value: DatumspaceError) -> Self {
        Self::DatumspaceError(value)
    }
}

impl From<AllocatorError> for LoaderError
{
    fn from(value: AllocatorError) -> Self {
        Self::AllocatorError(value)
    }
}

impl<'a> Loader<'a>
{
    pub fn new(base: &str) -> Result<Self, LoaderError>
    {
        let mut datumspace = Datumspace::with_capacity(DEFAULT_CAPACITY)?;

        // Load initial page
        let parsed_file = parse_file(Path::new(base))?;
        let initial_page = datumspace.load_datum(&parsed_file)?;

        Ok(
            Self {
                datumspace,
                base: *initial_page.id
            }
        )
    }

    fn load_file(&mut self, path: &Path) -> Result<DatumPage<'a>, LoaderError>
    {
        // Load initial page
        let parsed_file = parse_file(path)?;
        let page = self.datumspace.load_datum(&parsed_file)?;

        Ok(page)
    }

    pub fn load_link(&mut self, from: &SymbolId, link: &Link) -> Result<DatumPage<'a>, LoaderError>
    {
        // First check if the link is already loaded, in which case just fetch it
        match self.datumspace.get_page(&link.id)
        {
            Ok(page) => Ok(page),
            Err(DatumspaceError::PageNotLoaded) => Ok(
                // Load the file, then verify the correct module was loaded
                self.load_file(
                    Path::new(self.datumspace.resolve_string(from, &link.path)?)
                )
                .and_then(|x| (*x.id == link.id).then_some(x).ok_or(LoaderError::InvalidLink))?
            ),
            _ => Err(LoaderError::FailedToFindSymbol),
        }
    }

    pub fn initial_context<'b>(&'b mut self) -> Result<LoaderContext<'b, 'a>, LoaderError>
    {
        LoaderContext::new(self, self.base)
    }
}

pub struct LoaderContext<'a, 'b>
{
    loader: &'a mut Loader<'b>,
    page_id: SymbolId,
    page: DatumPage<'b>
}

impl<'a, 'b> LoaderContext<'a, 'b>
{
    pub fn new(loader: &'a mut Loader<'b>, id: SymbolId) -> Result<Self, LoaderError>
    {
        let base = loader.datumspace.get_page(&id)?;

        Ok(
            LoaderContext {
                loader: loader,
                page_id: id,
                page: base,
            }
        )
    }

    pub fn with_link<F, T>(&'a mut self, link_index: usize, func: F) -> Result<T, LoaderError>
    where
        F: FnOnce(Self) -> T
    {
        let link = self.page.links.get(link_index).ok_or(LoaderError::FailedToFindSymbol)?;
        let page = self.loader.load_link(&self.page_id, link)?;

        Ok(
            func(
                LoaderContext { loader: self.loader, page_id: *page.id, page }
            )
        )
    }

    /// Special case of `get_function_by_flags` for one of its most common use cases
    ///
    /// NOTE: This does not check whether the entrypoint found is unique, but rather
    /// just finds the first function marked as a entrypoint.
    /// As having multiple entrypoints in the same file is classified as
    /// Undefined Behaviour, this shouldn't happen anyway
    pub fn get_entrypoint(&'a self) -> Result<Option<FunctionInfo<'a>>, LoaderError>
    {
        self.get_function_by_flags(FunctionFlags::ENTRYPOINT)
    }

    /// Finds the first function to match the given flags
    ///
    /// If no functions with those flags exists, returns Ok(None),
    /// otherwise will either return the first function found,
    /// or will
    pub fn get_function_by_flags(&'a self, flags: FunctionFlags) -> Result<Option<FunctionInfo<'a>>, LoaderError>
    {
        self.page
            .functions
            .iter()
            .find_map(|x| match x {
                Runnable::Function(f) if f.flags == flags => Some(f),
                _ => None,
            })
            .map(|entrypoint| FunctionInfo::from_datumspace(&self.loader.base, &self.loader.datumspace, entrypoint))
            .transpose()
    }

    pub fn get_function(&'a self, index: usize) -> Result<FunctionInfo<'a>, LoaderError>
    {
        self.page.functions
            .get(index)
            .ok_or(LoaderError::FailedToFindSymbol)
            .and_then(|func| match func {
                Runnable::Function(f) => FunctionInfo::from_datumspace(&self.page_id, &self.loader.datumspace, f)
            })
    }

    pub fn get_constant(&mut self, index: usize) -> Result<Constant, LoaderError>
    {
        self.loader.datumspace
            .get_constant(&self.page_id, index)
            .map_err(LoaderError::DatumspaceError)
            .copied()
    }
}

// Wrapper Structs


pub struct FunctionInfo<'a>
{
    maxstack: usize,
    maxlocals: usize,
    bytecode: &'a [u8]
}

impl<'a> FunctionInfo<'a>
{
    pub fn from_datumspace(base: &SymbolId, datumspace: &'a Datumspace, function: &Function) -> Result<Self, LoaderError>
    {
        // Extract important information, and resolve code location
        let (maxstack, maxlocals) = function.setup_info();
        let bytecode = datumspace.resolve_location(base, function.bytecode)?;

        Ok(
            FunctionInfo { maxstack, maxlocals, bytecode }
        )
    }

    pub fn setup_info(&self) -> (usize, usize)
    {
        (self.maxstack, self.maxlocals)
    }

    /// Get the bytecode of a function
    ///
    /// This is unsafe basically because otherwise the borrow checker doesn't
    /// understand this is fine.
    /// This code slice is ultimately stored in Datumspace somewhere, and so its perfectly
    /// safe to use and won't get randomly dropped.
    pub fn code(&self) -> &'static [u8]
    {
        // Very dodgy looking but trust me broz
        // If something starts going wrong, THIS is the first place to look
        unsafe { transmute(self.bytecode) }
    }
}
