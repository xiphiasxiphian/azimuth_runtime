use std::ptr::NonNull;

#[derive(Clone, Copy)]
pub enum StackEntry
{
    Unsigned(usize),
    Signed(isize),
    Character(char),
    Boolean(bool),
    Float(f32),
    Double(f64),
    Reference(Option<NonNull<u8>>)
}
