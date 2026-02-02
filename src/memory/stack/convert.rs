// The narrowing primitive conversion behaviour here is desired
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]

use crate::memory::stack::entry::StackEntry;

/// Defines behaviour of converting between stack types
pub trait StackableConvert<T: Into<StackEntry>>: Into<StackEntry>
{
    fn convert(from: T) -> Self;
}

macro_rules! impl_convert {
    { $($from:ty => $to:ty),* } => {
        $(
            impl StackableConvert<$from> for $to
            {
                fn convert(from: $from) -> Self
                {
                    from as Self
                }
            }
        )*
    };
}

// Using i64 to avoid sign loss
impl_convert! {
    usize => isize,
    isize => usize,
    f32 => isize,
    f64 => isize,
    isize => f32,
    f64 => f32,
    isize => f64,
    f32 => f64
}
