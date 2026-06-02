use crate::{
    loader::parser::layout::{ScalarTag, TypeSignature},
    memory::datumspace::DatumspaceError,
};

/// An 8-byte layout cache. No vectors, no heap allocations.
#[derive(Clone, Copy, Default, Debug)]
pub struct TypeLayout
{
    pub size: u32,
    pub align: u32,
    pub has_gc_roots: bool,
}

pub struct LayoutEngine
{
    resolved: Vec<TypeLayout>,
    pointer_size: u32,
}

impl LayoutEngine
{
    pub fn new(type_count: usize) -> Self
    {
        Self {
            resolved: vec![TypeLayout::default(); type_count],
            pointer_size: size_of::<usize>() as u32,
        }
    }

    /// Fast bitwise alignment intrinsic
    #[inline(always)]
    pub fn align_to(offset: u32, align: u32) -> u32
    {
        (offset + align - 1) & !(align - 1)
    }

    /// Resolves fields linearly, writing GC offsets directly to the global buffer.
    pub fn resolve_fields(
        &self,
        fields: &[TypeSignature], // Assuming TypeSignature is in scope
        global_gc_offsets: &mut Vec<usize>,
        base_offset: u32,
    ) -> Result<TypeLayout, DatumspaceError>
    {
        let mut current_offset = base_offset;
        let mut max_align = 1;
        let mut has_gc_roots = false;

        for sig in fields
        {
            let (field_size, field_align, is_gc_root) = match sig
            {
                TypeSignature::Scalar(tag) => match tag
                {
                    ScalarTag::Integer32 | ScalarTag::Float32 => (4, 4, false),
                    ScalarTag::Integer64 | ScalarTag::Float64 => (8, 8, false),
                },
                TypeSignature::String | TypeSignature::Reference { .. } => (self.pointer_size, self.pointer_size, true),
                TypeSignature::ValueType { type_index } =>
                {
                    // O(1) lookup. Bounds check inherently handles invalid indices.
                    let cached = self
                        .resolved
                        .get(*type_index as usize)
                        .ok_or(DatumspaceError::InvalidStructure)?;

                    // Enforce your Loader Invariant at lightning speed
                    if cached.has_gc_roots
                    {
                        return Err(DatumspaceError::InvalidStructure);
                    }
                    (cached.size, cached.align, false)
                }
            };

            current_offset = Self::align_to(current_offset, field_align);

            if is_gc_root
            {
                global_gc_offsets.push(current_offset as usize);
                has_gc_roots = true;
            }

            current_offset += field_size;
            max_align = max_align.max(field_align);
        }

        let final_size = Self::align_to(current_offset, max_align);

        Ok(TypeLayout {
            size: final_size,
            align: max_align,
            has_gc_roots,
        })
    }
}
