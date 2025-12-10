use crate::engine::stack::stackable::Stackable;

/// Defines behaviour of converting between stack types
pub trait StackableConvert<T: Stackable> : Stackable
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

impl_convert! {
    f32 => u64,
    f64 => u64,
    u64 => f32,
    f64 => f32,
    u64 => f64,
    f32 => f64
}
