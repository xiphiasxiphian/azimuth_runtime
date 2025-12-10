use crate::engine::stack::StackEntry;

pub trait Stackable: Copy
{
    fn into_entry(self) -> StackEntry;
    fn from_entry(entry: StackEntry) -> Self;
}

impl Stackable for StackEntry
{
    fn into_entry(self) -> StackEntry
    {
        self
    }

    fn from_entry(entry: StackEntry) -> Self
    {
        entry
    }
}

impl Stackable for i64
{
    fn into_entry(self) -> StackEntry
    {
        // The compiler should be intelligent enough to realise this is a no-op
        <StackEntry>::from_le_bytes(self.to_le_bytes())
    }

    fn from_entry(entry: StackEntry) -> Self
    {
        // The compiler should be intelligent enough to realise this is a no-op
        Self::from_le_bytes(entry.to_le_bytes())
    }
}

impl Stackable for u32
{
    fn into_entry(self) -> StackEntry
    {
        self.into()
    }

    #[expect(clippy::cast_possible_truncation, reason = "Truncating behaviour here is desired")]
    fn from_entry(entry: StackEntry) -> Self
    {
        entry as Self // Truncating behavior desired
    }
}

impl Stackable for f32
{
    fn into_entry(self) -> StackEntry
    {
        StackEntry::from(self.to_bits())
    }

    #[expect(clippy::cast_possible_truncation, reason = "Truncating behaviour here is desired")]
    fn from_entry(entry: StackEntry) -> Self
    {
        Self::from_bits(entry as u32) // The truncating behaviour here is desired
    }
}

impl Stackable for f64
{
    fn into_entry(self) -> StackEntry
    {
        self.to_bits()
    }

    fn from_entry(entry: StackEntry) -> Self
    {
        Self::from_bits(entry)
    }
}

impl<T> Stackable for *const T
{
    fn into_entry(self) -> StackEntry
    {
        self as StackEntry
    }

    fn from_entry(entry: StackEntry) -> Self
    {
        entry as Self
    }
}
