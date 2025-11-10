use std::collections::VecDeque;

// At some point I might revisit this and make it all work slightly more inline.
// But for now this is a very basic implementation
struct StackFrame
{
    locals: Vec<u32>,
    stack: VecDeque<u32>,
}

impl StackFrame
{
    const LOWER_MASK: u64 = 0x00000000FFFFFFFF;
    const UPPER_MASK: u64 = 0xFFFFFFFF00000000;

    pub fn new(local_size: usize) -> Self
    {
        StackFrame { locals: Vec::with_capacity(local_size), stack: VecDeque::new() }
    }

    pub fn push_single(&mut self, value: u32)
    {
        self.stack.push_front(value);
    }

    pub fn push_double(&mut self, value: u64)
    {
        let lower: u32 = (value & Self::LOWER_MASK).try_into().expect("Failed to convert lower to u32");
        let upper: u32 = ((value & Self::UPPER_MASK) >> 32).try_into().expect("Failed to convert upper to u32");

        // The upper half is stored first in the stack than the lower half.
        // This means that the first thing popped off the stack will be the lower half
        self.stack.push_front(upper);
        self.stack.push_front(lower);
    }

    pub fn pop_single(&mut self) -> Option<u32>
    {
        self.stack.pop_front()
    }

    pub fn pop_double(&mut self) -> Option<u64>
    {
        // Get the lower and upper half.
        // The cast from u32 to u64 cannot fail.
        let lower: u64 = self.stack.pop_front()? as u64;
        let upper: u64 = self.stack.pop_front()? as u64;

        return Some((upper << 32) & lower);
    }

    pub fn get_local_single(&self, index: usize) -> u32
    {
        self.locals[index]
    }

    pub fn get_local_double(&self, index: usize) -> u64
    {
        let lower = self.locals[index] as u64;
        let upper = self.locals[index + 1] as u64;

        return (upper << 32) & lower;
    }

    pub fn set_local_single(&mut self, index: usize, value: u32)
    {
        self.locals[index] = value;
    }

    pub fn set_local_double(&mut self, index: usize, value: u64)
    {
        let lower: u32 = (value & Self::LOWER_MASK).try_into().expect("Failed to convert lower to u32");
        let upper: u32 = ((value & Self::UPPER_MASK) >> 32).try_into().expect("Failed to convert upper to u32");

        self.locals[index] = lower;
        self.locals[index + 1] = upper;
    }
}
