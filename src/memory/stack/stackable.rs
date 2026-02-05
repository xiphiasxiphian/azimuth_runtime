use std::ptr::NonNull;
use crate::memory::stack::{entry::StackEntry};

pub struct EntryExtractionError;
macro_rules! impl_into_entry {
    ($($t:ty => $r:tt),+) => {
        $(
            impl From<$t> for StackEntry
            {
                fn from(value: $t) -> Self { Self::$r(value) }
            }

            impl TryFrom<StackEntry> for $t
            {
                type Error = EntryExtractionError;

                fn try_from(value: StackEntry) -> Result<Self, Self::Error>
                {
                    match value
                    {
                        StackEntry::$r(x) => Ok(x),
                        _ => Err(EntryExtractionError)
                    }
                }
            }
        )+
    };
}

impl_into_entry!(
    u64 => Unsigned,
    i64 => Signed,
    char => Character,
    f32 => Float,
    f64 => Double,
    Option<NonNull<u8>> => Reference
);
