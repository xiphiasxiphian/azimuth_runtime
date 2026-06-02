use crate::{loader::SymbolId, memory::datumspace::datum::Offset};

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub enum RuntimeTypeKind {
    Struct {
        /// Total byte size to allocate on the heap (including ObjectHeader)
        instance_size: usize,
        /// Start index into the page's `gc_offsets` array
        gc_offsets_index: u32,
        gc_offsets_count: u32,
    },
    Enum {
        /// Start index into the page's `enum_variants` array
        variants_index: u32,
        variants_count: u32,
    },
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RuntimeType {
    pub symbol_id: SymbolId,
    pub kind: RuntimeTypeKind,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RuntimeEnumVariant {
    pub tag: u32,
    /// Allocation size needed specifically for this variant payload + header
    pub instance_size: usize,
    /// Start index into the page's `gc_offsets` array
    pub gc_offsets_index: u32,
    pub gc_offsets_count: u32,
}
