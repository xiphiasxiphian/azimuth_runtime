use std::{cell::UnsafeCell, ptr::NonNull};

// This is a more formalised wrapper around the idea of the constant table.
//
use crate::{
    loader::parser::layout::{ConstantSignature, ScalarTag},
    memory::{
        datumspace::datum::{BlockLocation, InlinedString},
        stack::entry::StackEntry,
    },
};

pub type ConstantTableIndex = u32;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DataEntry
{
    pub loc: BlockLocation,
    pub tag: ConstantSignature,
}

#[derive(Clone, Copy, Debug)]
pub enum ConstantTableEntryData
{
    Unresolved(DataEntry),
    Resolved(Constant),
}

pub struct ConstantTableEntry
{
    inner: UnsafeCell<ConstantTableEntryData>,
}

impl ConstantTableEntry
{
    pub fn new(data: ConstantTableEntryData) -> Self
    {
        Self {
            inner: UnsafeCell::new(data),
        }
    }

    // Safe read access: Converts the cell to a shared reference.
    // SAFETY: You must ensure no other code is actively writing to this
    // specific entry at the exact moment this is called.
    pub fn as_data(&self) -> &ConstantTableEntryData
    {
        unsafe { &*self.inner.get() }
    }

    // Safe/Unsafe write access depending on your thread-safety guarantees
    pub unsafe fn set_data(&self, new_data: ConstantTableEntryData)
    {
        let ptr = self.inner.get();
        unsafe { ptr.write(new_data) };
    }
}

/// A Constant stored within the constant table.
///
/// These are roughly direct copies of the ones stored within
/// the binary itself, but abstracted out for the sake
/// of ease of use.
///
/// ## Variants
/// There are 5 main types of Constant:
///
/// `Unsigned32` - Stores a `u32` (also called `int` in some languages).
/// It is important to note that this exists
/// out of pure convenience, as it will get padded up to 64-bits when loaded
/// onto the stack.
///
/// `Unsigned64` - Stores a `u64` (also called `long` in some languages).
///
/// `Float32` - Stores a `f32`. It is important to note that, unlike `Unsigned32`,
/// this doesn't exist just for convenience, as sometimes representing different
/// floating point precisions can be important. However, like `Unsigned32`, this will
/// still get padded with 0s to 64-bits when loaded onto the stack.
///
/// `Float64` - Stores a `f64` (also called `double` in some languages)
///
/// `String` - Stores a string reference (the string data is stored in metaspace)
#[derive(Debug, Clone, Copy)]
pub enum Constant
{
    Unsigned32(u32),
    Unsigned64(u64),
    Float32(f32),
    Float64(f64),
    String(InlinedString),
}

impl From<Constant> for StackEntry
{
    fn from(value: Constant) -> Self
    {
        match value
        {
            Constant::Unsigned32(x) => <u64>::from(x).into(),
            Constant::Unsigned64(x) => x.into(),
            Constant::Float32(x) => x.into(),
            Constant::Float64(x) => x.into(),
            Constant::String(_x) => todo!(), // How does the possibly not pinned string get translated here
        }
    }
}

impl Constant
{
    pub unsafe fn from_entry(base: NonNull<u8>, DataEntry { loc, tag }: &DataEntry) -> Option<Self>
    {
        let bytes: &[u8] = unsafe {
            let ptr: NonNull<u8> = loc.0.as_ptr(base);
            NonNull::slice_from_raw_parts(ptr, loc.1 as usize).as_ref()
        };

        let constant = match tag
        {
            ConstantSignature::Scalar(ScalarTag::Integer32) =>
            {
                Constant::Unsigned32(<u32>::from_le_bytes(*(bytes.first_chunk()?)))
            }
            ConstantSignature::Scalar(ScalarTag::Integer64) =>
            {
                Constant::Unsigned64(<u64>::from_le_bytes(*(bytes.first_chunk()?)))
            }
            ConstantSignature::Scalar(ScalarTag::Float32) =>
            {
                Constant::Float32(<f32>::from_bits(<u32>::from_le_bytes(*(bytes.first_chunk()?))))
            }
            ConstantSignature::Scalar(ScalarTag::Float64) =>
            {
                Constant::Float64(<f64>::from_bits(<u64>::from_le_bytes(*(bytes.first_chunk()?))))
            }
            ConstantSignature::String => Constant::String(InlinedString::new(*loc)),
            ConstantSignature::ValueType { type_index: _ } => todo!("Value type constants not implemented"),
        };

        Some(constant)
    }
}
