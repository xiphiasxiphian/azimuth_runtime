use crate::memory::datumspace::{constant_table::Constant, runnable::Runnable};

#[repr(C)]
pub struct DatumPageHeader
{
    pub id_len: u32,
    pub constants_len: u32,
    pub functions_len: u32,
}

impl DatumPageHeader
{
    pub unsafe fn get_page(&self) -> DatumPage
    {
        unsafe { DatumPage::from_base_ptr(self as *const _ as *const u8) }
    }
}

/// A typed view over a raw DatumPage block.
/// All slices point into the same contiguous allocation.
pub struct DatumPage<'a>
{
    pub id: &'a str,
    pub constants: &'a [Constant<'a>],
    pub functions: &'a [Runnable<'a>],
}

impl<'a> DatumPage<'a>
{
    /// Reinterpret the base pointer of a filled page block.
    /// Safe only if the block was built by `load_datum`.
    pub unsafe fn from_base_ptr(ptr: *const u8) -> Self
    {
        let header = unsafe { &*(ptr as *const DatumPageHeader) };

        let id_offset = size_of::<DatumPageHeader>();
        let const_offset = align_up(id_offset + header.id_len as usize, align_of::<Constant>());
        let fn_offset = align_up(
            const_offset + header.constants_len as usize * size_of::<Constant>(),
            align_of::<Runnable>(),
        );

        let id =
            unsafe { str::from_utf8_unchecked(std::slice::from_raw_parts(ptr.add(id_offset), header.id_len as usize)) };

        let constants = unsafe {
            std::slice::from_raw_parts(ptr.add(const_offset) as *const Constant, header.constants_len as usize)
        };

        let functions =
            unsafe { std::slice::from_raw_parts(ptr.add(fn_offset) as *const Runnable, header.functions_len as usize) };

        DatumPage {
            id,
            constants,
            functions,
        }
    }
}

pub fn align_up(offset: usize, align: usize) -> usize
{
    (offset + align - 1) & !(align - 1)
}
