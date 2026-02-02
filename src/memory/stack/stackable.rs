use std::ptr::NonNull;
use crate::memory::stack::entry::StackEntry;

macro_rules! impl_into_entry {
    ($($t:ty => $r:tt),+) => {
        $(
            impl From<$t> for StackEntry
            {
                fn from(value: $t) -> Self { Self::$r(value) }
            }
        )+
    };
}

impl_into_entry!(
    usize => Unsigned,
    isize => Signed,
    char => Character,
    bool => Boolean,
    f32 => Float,
    f64 => Double,
    Option<NonNull<u8>> => Reference
);
