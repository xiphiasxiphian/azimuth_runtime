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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::parser::layout::{FieldDef, ScalarTag, TypeSignature};
    use crate::memory::datumspace::DatumspaceError;
    use std::mem::size_of;

    // --- Helper functions for cleaner test setups ---

    fn mock_field(signature: TypeSignature) -> FieldDef {
        FieldDef {
            // Assuming name exists but is ignored by the engine logic
            name: 0,
            signature,
        }
    }

    fn ptr_size() -> u32 {
        size_of::<usize>() as u32
    }

    // --- 1. Core Mathematics ---

    #[test]
    fn test_align_to() {
        assert_eq!(LayoutEngine::align_to(0, 4), 0);
        assert_eq!(LayoutEngine::align_to(1, 4), 4);
        assert_eq!(LayoutEngine::align_to(3, 4), 4);
        assert_eq!(LayoutEngine::align_to(4, 4), 4);

        assert_eq!(LayoutEngine::align_to(5, 8), 8);
        assert_eq!(LayoutEngine::align_to(8, 8), 8);
        assert_eq!(LayoutEngine::align_to(9, 8), 16);
    }

    #[test]
    fn test_engine_initialization() {
        let engine = LayoutEngine::new(10);
        assert_eq!(engine.resolved.len(), 10);
        assert_eq!(engine.pointer_size, ptr_size());
    }

    // --- 2. Field Resolution (Scalars & Alignment) ---

    #[test]
    fn test_resolve_fields_scalars() {
        let engine = LayoutEngine::new(0);
        let fields = vec![
            mock_field(TypeSignature::Scalar(ScalarTag::Integer32)), // Offset 0, size 4
            mock_field(TypeSignature::Scalar(ScalarTag::Integer64)), // Offset 8 (padded), size 8
            mock_field(TypeSignature::Scalar(ScalarTag::Float32)),   // Offset 16, size 4
        ];

        let mut gc_offsets = Vec::new();
        let layout = engine.resolve_fields(&fields, &mut gc_offsets, 0).unwrap();

        // Total size should be 24 (padded to align 8 at the end)
        // 0..4 (I32) -> 4..8 (pad) -> 8..16 (I64) -> 16..20 (F32) -> 20..24 (pad to max align 8)
        assert_eq!(layout.size, 24);
        assert_eq!(layout.align, 8);
        assert!(!layout.has_gc_roots);
        assert!(gc_offsets.is_empty());
    }

    // --- 3. Field Resolution (GC Roots & Base Offsets) ---

    #[test]
    fn test_resolve_fields_gc_roots() {
        let engine = LayoutEngine::new(0);
        let fields = vec![
            mock_field(TypeSignature::Scalar(ScalarTag::Integer32)),
            mock_field(TypeSignature::String), // Pointer sized, GC tracked
            mock_field(TypeSignature::Reference { type_index: 99 }), // Pointer sized, GC tracked
        ];

        let mut gc_offsets = Vec::new();
        // Base offset of 16 simulates an object header
        let base_offset = 16;

        let layout = engine.resolve_fields(&fields, &mut gc_offsets, base_offset).unwrap();

        let ptr = ptr_size();

        // I32 at 16..20. String aligned to `ptr` (24 on 64-bit, 20 on 32-bit).
        let expected_string_offset = LayoutEngine::align_to(base_offset + 4, ptr);
        let expected_ref_offset = expected_string_offset + ptr;

        assert!(layout.has_gc_roots);
        assert_eq!(layout.align, ptr);
        assert_eq!(gc_offsets.len(), 2);
        assert_eq!(gc_offsets[0], expected_string_offset as usize);
        assert_eq!(gc_offsets[1], expected_ref_offset as usize);
    }

    // --- 4. Value Types & Error Handling ---

    #[test]
    fn test_resolve_value_type_success() {
        let mut engine = LayoutEngine::new(2);
        // Prime the cache with a known value type layout (no GC roots)
        engine.resolved[1] = TypeLayout { size: 12, align: 4, has_gc_roots: false };

        let fields = vec![
            mock_field(TypeSignature::ValueType { type_index: 1 }),
        ];

        let mut gc_offsets = Vec::new();
        let layout = engine.resolve_fields(&fields, &mut gc_offsets, 0).unwrap();

        assert_eq!(layout.size, 12);
        assert_eq!(layout.align, 4);
        assert!(!layout.has_gc_roots);
    }

    #[test]
    fn test_resolve_value_type_with_gc_roots_fails() {
        let mut engine = LayoutEngine::new(2);
        // A value type that contains GC roots (e.g. holds a String)
        engine.resolved[1] = TypeLayout { size: 8, align: 8, has_gc_roots: true };

        let fields = vec![
            mock_field(TypeSignature::ValueType { type_index: 1 }),
        ];

        let mut gc_offsets = Vec::new();
        let result = engine.resolve_fields(&fields, &mut gc_offsets, 0);

        // Cannot embed GC roots inside value types according to engine rules
        assert!(matches!(result, Err(DatumspaceError::InvalidStructure)));
    }

    #[test]
    fn test_resolve_value_type_invalid_index() {
        let engine = LayoutEngine::new(1); // Only index 0 exists

        let fields = vec![
            mock_field(TypeSignature::ValueType { type_index: 99 }), // Out of bounds
        ];

        let mut gc_offsets = Vec::new();
        let result = engine.resolve_fields(&fields, &mut gc_offsets, 0);

        assert!(matches!(result, Err(DatumspaceError::InvalidStructure)));
    }

    // --- 5. Struct Resolution ---

    #[test]
    fn test_resolve_struct() {
        let mut engine = LayoutEngine::new(1);
        let fields = vec![
            mock_field(TypeSignature::Scalar(ScalarTag::Integer64)),
        ];

        let mut gc_offsets = Vec::new();
        let header_size = 16;
        let type_index = 0;

        let heap_layout = engine.resolve_struct(type_index, &fields, &mut gc_offsets, header_size).unwrap();

        // The cached inline layout should have 0 base offset
        let inline_layout = engine.resolved[type_index];
        assert_eq!(inline_layout.size, 8);
        assert_eq!(inline_layout.align, 8);

        // The returned heap layout should include the header base offset
        // 16 (header) + 8 (I64) = 24
        assert_eq!(heap_layout.size, 24);
        assert_eq!(heap_layout.align, 8);
    }

    // --- 6. Enum/Variant Resolution ---

    #[test]
    fn test_enum_variants_and_finalization() {
        let mut engine = LayoutEngine::new(1);
        let header_size = 16;

        // Variant 1: Just an I32
        let fields_v1 = vec![mock_field(TypeSignature::Scalar(ScalarTag::Integer32))];
        let mut gc_offsets_v1 = Vec::new();
        let layout_v1 = engine.resolve_variant(&fields_v1, &mut gc_offsets_v1, header_size).unwrap();

        // Variant 2: An I64
        let fields_v2 = vec![mock_field(TypeSignature::Scalar(ScalarTag::Integer64))];
        let mut gc_offsets_v2 = Vec::new();
        let layout_v2 = engine.resolve_variant(&fields_v2, &mut gc_offsets_v2, header_size).unwrap();

        // Check Variant 1 Internal Logic (4 byte discriminant)
        // Inline: 4 (disc) + 4 (I32) = 8
        assert_eq!(layout_v1.inline.size, 8);
        // Heap: 16 (header) + 4 (disc) + 4 (I32) = 24
        assert_eq!(layout_v1.heap.size, 24);

        // Check Variant 2 Internal Logic (Requires Padding)
        // Inline: 4 (disc) -> Pad to 8 -> 8 (I64) = 16
        assert_eq!(layout_v2.inline.size, 16);
        // Heap: 16 (header) + 4 (disc) -> Pad to 24 -> 8 (I64) = 32
        assert_eq!(layout_v2.heap.size, 32);

        // Finalize Enum
        let variants = vec![layout_v1, layout_v2];
        let type_index = 0;
        let final_heap_layout = engine.finalize_enum(type_index, &variants);

        // Overall enum aligns to the strictest requirement (I64 -> 8)
        assert_eq!(final_heap_layout.align, 8);

        // Final heap size should max out at Variant 2's size
        assert_eq!(final_heap_layout.size, 32);

        // Final inline size cached should max out at Variant 2's inline size
        let final_inline_layout = engine.resolved[type_index];
        assert_eq!(final_inline_layout.size, 16);
        assert_eq!(final_inline_layout.align, 8);
    }
}
