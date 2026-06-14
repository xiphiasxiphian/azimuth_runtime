use crate::{
    common::VecSet,
    loader::parser::layout::{FieldDef, ScalarTag, TypeSignature},
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

#[derive(Clone, Copy, Debug)]
pub struct VariantLayout
{
    pub inline: TypeLayout,
    pub heap: TypeLayout,
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
    pub fn align_to(offset: u32, align: u32) -> u32
    {
        (offset + align - 1) & !(align - 1)
    }

    pub fn cache_result(&mut self, layout: TypeLayout, index: usize) -> bool
    {
        self.resolved.set(layout, index).is_some()
    }

    /// Resolves fields linearly, writing GC offsets directly to the global buffer.
    pub fn resolve_fields(
        &self,
        fields: &[FieldDef],
        global_gc_offsets: &mut Vec<usize>,
        base_offset: u32,
    ) -> Result<TypeLayout, DatumspaceError>
    {
        let mut current_offset = base_offset;
        let mut max_align = 1;
        let mut has_gc_roots = false;

        for FieldDef {
            name: _,
            signature: sig,
        } in fields
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
                    let cached = self
                        .resolved
                        .get(*type_index as usize)
                        .ok_or(DatumspaceError::InvalidStructure)?;

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

    /// Resolves a struct's inline and heap layout, writing to the gc buffer and caching the footprint.
    pub fn resolve_struct(
        &mut self,
        type_index: usize,
        fields: &[FieldDef],
        global_gc_offsets: &mut Vec<usize>,
        header_size: u32,
    ) -> Result<TypeLayout, DatumspaceError>
    {
        // computes zero-offset inline layout for value type metadata cache
        let mut dummy_offsets = Vec::new();
        let inline_layout = self.resolve_fields(fields, &mut dummy_offsets, 0)?;
        self.resolved[type_index] = inline_layout;

        // computes heap-offset layout for physical allocation and gc tracking
        let heap_layout = self.resolve_fields(fields, global_gc_offsets, header_size)?;
        Ok(heap_layout)
    }

    /// Resolves an individual variant's layout without committing to the type cache.
    pub fn resolve_variant<'a>(
        &self,
        fields: &[FieldDef],
        global_gc_offsets: &mut Vec<usize>,
        header_size: u32,
    ) -> Result<VariantLayout, DatumspaceError>
    {
        // variant fields inside inline contexts start immediately after the 4-byte discriminant
        let mut dummy_offsets = Vec::new();
        let inline = self.resolve_fields(fields, &mut dummy_offsets, 4)?;

        // variant fields on the heap start after both the object header and the 4-byte discriminant
        let heap = self.resolve_fields(fields, global_gc_offsets, header_size + 4)?;

        Ok(VariantLayout { inline, heap })
    }

    /// Consolidates all variant layouts to determine the overall enum bounds and commits it to the cache.
    pub fn finalize_enum(&mut self, type_index: usize, variants: &[VariantLayout]) -> TypeLayout
    {
        let mut max_inline_size = 4;
        let mut max_heap_size = 4;
        let mut max_align = 4;
        let mut has_gc_roots = false;

        for v in variants
        {
            max_inline_size = max_inline_size.max(v.inline.size);
            max_heap_size = max_heap_size.max(v.heap.size);
            max_align = max_align.max(v.inline.align);
            has_gc_roots |= v.inline.has_gc_roots;
        }

        // pads the overall enum sizes up to the strictest alignment constraint found
        let final_inline_size = Self::align_to(max_inline_size, max_align);
        let final_heap_size = Self::align_to(max_heap_size, max_align);

        self.resolved[type_index] = TypeLayout {
            size: final_inline_size,
            align: max_align,
            has_gc_roots,
        };

        TypeLayout {
            size: final_heap_size,
            align: max_align,
            has_gc_roots,
        }
    }
}
