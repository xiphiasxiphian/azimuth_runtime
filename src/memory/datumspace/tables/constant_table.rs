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
            Constant::String(_x) => todo!(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::NonNull;
    use crate::{loader::parser::layout::{ConstantSignature, ScalarTag}, memory::datumspace::datum::Offset};

    // --- Helper Functions ---

    /// Creates a mock DataEntry.
    /// Note: You may need to adjust the initialization of `loc` depending on
    /// the exact tuple/struct definition of `BlockLocation` in your crate.
    fn mock_data_entry(offset: u32, length: u32, tag: ConstantSignature) -> DataEntry {
        DataEntry {
            loc: (Offset(offset), length),
            tag,
        }
    }

    // --- 1. Interior Mutability Tests ---

    #[test]
    fn test_constant_table_entry_mutability() {
        // 1. Initialize with unresolved data
        let dummy_entry = mock_data_entry(0, 4, ConstantSignature::Scalar(ScalarTag::Integer32));
        let table_entry = ConstantTableEntry::new(ConstantTableEntryData::Unresolved(dummy_entry));

        // 2. Take an IMMUTABLE reference
        let entry_ref = &table_entry;

        // 3. Verify initial state
        assert!(matches!(entry_ref.as_data(), ConstantTableEntryData::Unresolved(_)));

        // 4. Mutate through the immutable reference safely
        unsafe {
            entry_ref.set_data(ConstantTableEntryData::Resolved(Constant::Unsigned32(42)));
        }

        // 5. Verify the new state took effect
        match entry_ref.as_data() {
            ConstantTableEntryData::Resolved(Constant::Unsigned32(val)) => assert_eq!(*val, 42),
            _ => panic!("Expected Resolved(Constant::Unsigned32)"),
        }
    }

    // --- 2. Raw Memory Parsing Tests (from_entry) ---

    #[test]
    fn test_parse_unsigned32() {
        // 0x12345678 in Little Endian
        let mut buffer: Vec<u8> = vec![0x78, 0x56, 0x34, 0x12];
        let base = NonNull::new(buffer.as_mut_ptr()).unwrap();

        let entry = mock_data_entry(0, 4, ConstantSignature::Scalar(ScalarTag::Integer32));

        let constant = unsafe { Constant::from_entry(base, &entry).unwrap() };

        assert!(matches!(constant, Constant::Unsigned32(0x12345678)));
    }

    #[test]
    fn test_parse_unsigned64() {
        // 0x1122334455667788 in Little Endian
        let mut buffer: Vec<u8> = vec![0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];
        let base = NonNull::new(buffer.as_mut_ptr()).unwrap();

        let entry = mock_data_entry(0, 8, ConstantSignature::Scalar(ScalarTag::Integer64));

        let constant = unsafe { Constant::from_entry(base, &entry).unwrap() };

        assert!(matches!(constant, Constant::Unsigned64(0x1122334455667788)));
    }

    #[test]
    fn test_parse_float32() {
        // 3.14159f32 encoded in Little Endian bytes
        let mut buffer: Vec<u8> = 3.14159f32.to_le_bytes().to_vec();
        let base = NonNull::new(buffer.as_mut_ptr()).unwrap();

        let entry = mock_data_entry(0, 4, ConstantSignature::Scalar(ScalarTag::Float32));

        let constant = unsafe { Constant::from_entry(base, &entry).unwrap() };

        if let Constant::Float32(val) = constant {
            assert_eq!(val, 3.14159f32);
        } else {
            panic!("Expected Float32");
        }
    }

    #[test]
    fn test_parse_float64() {
        // 2.718281828459045f64 encoded in Little Endian bytes
        let mut buffer: Vec<u8> = 2.718281828459045f64.to_le_bytes().to_vec();
        let base = NonNull::new(buffer.as_mut_ptr()).unwrap();

        let entry = mock_data_entry(0, 8, ConstantSignature::Scalar(ScalarTag::Float64));

        let constant = unsafe { Constant::from_entry(base, &entry).unwrap() };

        if let Constant::Float64(val) = constant {
            assert_eq!(val, 2.718281828459045f64);
        } else {
            panic!("Expected Float64");
        }
    }

    #[test]
    fn test_parse_string() {
        let mut buffer: Vec<u8> = vec![0; 8]; // Buffer doesn't matter for string initialization here
        let base = NonNull::new(buffer.as_mut_ptr()).unwrap();

        let entry = mock_data_entry(0, 8, ConstantSignature::String);

        let constant = unsafe { Constant::from_entry(base, &entry).unwrap() };

        assert!(matches!(constant, Constant::String(_)));
    }

    // --- 3. Stack Conversion Tests ---

    #[test]
    fn test_stack_entry_conversions() {
        let u32_const = Constant::Unsigned32(u32::MAX);
        let _stack_u32: StackEntry = u32_const.into();

        let u64_const = Constant::Unsigned64(u64::MAX);
        let _stack_u64: StackEntry = u64_const.into();

        let f32_const = Constant::Float32(1.0);
        let _stack_f32: StackEntry = f32_const.into();

        let f64_const = Constant::Float64(1.0);
        let _stack_f64: StackEntry = f64_const.into();
    }
}
