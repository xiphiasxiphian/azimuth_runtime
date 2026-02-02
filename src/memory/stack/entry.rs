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

impl StackEntry
{
    fn try_binary_operation<T, F>(&self, other: &Self, op: F) -> Option<Self>
    where
        Self: TryInto<T>,
        T: Into<StackEntry>,
        F: Fn(T, T) -> T
    {
        let first = self.try_into().ok()?;
        let second = other.try_into().ok()?;

        Some(op(first, second).into())
    }

    fn try_map<T, F>(&self, op: F) -> Option<Self>
    where
        Self: TryInto<T>,
        T: Into<StackEntry>,
        F: Fn(T) -> T
    {
        Some(op(self.try_into().ok()?).into())
    }
}
