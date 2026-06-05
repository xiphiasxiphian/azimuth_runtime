/// Stored in datumspace alongside the DatumPage.
/// `ObjectHeader::vtable_or_type` points to one of these.
/// Must be `#[repr(C)]` and pointer-stable.
#[repr(C)]
pub struct GcTypeDescriptor {
    /// Total heap allocation size including `ObjectHeader`.
    pub instance_size: usize,
    pub kind: GcDescriptorKind,
}

#[repr(C)]
pub enum GcDescriptorKind {
    Struct {
        /// Direct pointer into the page's `gc_offsets` slice. Valid for page lifetime.
        offsets_ptr: *const usize,
        offsets_len: usize,
    },
    Enum {
        /// Byte offset from object base to the 4-byte tag (= `size_of::<ObjectHeader>()`).
        tag_offset: usize,
        /// Direct pointer into an array of `GcVariantDescriptor`. Valid for page lifetime.
        variants_ptr: *const GcVariantDescriptor,
        variant_count: usize,
    },
}

#[repr(C)]
pub struct GcVariantDescriptor {
    pub tag: u32,
    pub offsets_ptr: *const usize,
    pub offsets_len: usize,
}

// Safety: the pointers are into datumspace which is pinned.
unsafe impl Send for GcTypeDescriptor {}
unsafe impl Sync for GcTypeDescriptor {}
