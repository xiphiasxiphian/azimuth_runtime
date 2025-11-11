use std::ops::Rem;

// Stack size is set at initiation and is hard coded somewhere.
// Theoretically this could become a config value at some point in the future
struct Stack<const N: usize>
{
    stack: [u32; N],
    head: usize
}

impl<const N: usize> Stack<N>
{
    pub fn new() -> Self
    {
        Stack { stack: [0; N], head: 0 }
    }

    pub fn push_frame<'a>(&'a mut self, locals_size: usize, stack_size: usize) -> Option<StackFrame<'a>>
    {
        let new_head = self.head + locals_size + stack_size;

        let (_, rem) = self.stack.split_at_mut(self.head);
        let (new, _) = rem.split_at_mut(locals_size + stack_size);

        let (locals, stack) = new.split_at_mut(locals_size);

        if new_head > N { return None; }
        self.head = new_head;

        Some(
            StackFrame::new(
                locals,
                stack
            )
        )
    }

    pub fn pop_stack(&mut self)
    {
        todo!()
    }
}

// At some point I might revisit this and make it all work slightly more inline.
// But for now this is a very basic implementation
#[derive(Debug)]
pub struct StackFrame<'a>
{
    locals: &'a mut [u32],
    stack: &'a mut [u32],
    stack_pointer: usize,
}

impl<'a> StackFrame<'a>
{
    const LOWER_MASK: u64 = 0x00000000FFFFFFFF;
    const UPPER_MASK: u64 = 0xFFFFFFFF00000000;

    pub fn new(locals: &'a mut [u32], stack: &'a mut [u32]) -> Self
    {
        StackFrame {
            locals: locals,
            stack: stack,
            stack_pointer: 0,
        }
    }

    pub fn push_single(&mut self, value: u32)
    {
        self.stack[self.stack_pointer] = value;
        self.stack_pointer += 1;
    }

    pub fn push_double(&mut self, value: u64)
    {
        let lower: u32 = (value & Self::LOWER_MASK)
            .try_into()
            .expect("Failed to convert lower to u32");
        let upper: u32 = ((value & Self::UPPER_MASK) >> 32)
            .try_into()
            .expect("Failed to convert upper to u32");

        // The upper half is stored first in the stack compared with the lower half.
        // This means that the first thing popped off the stack will be the lower half
        self.stack[self.stack_pointer] = upper;
        self.stack[self.stack_pointer + 1] = lower;

        self.stack_pointer += 2;
    }

    pub fn pop_single(&mut self) -> Option<u32>
    {
        if self.stack_pointer == 0 { return None; }

        self.stack_pointer -= 1;
        Some(self.stack[self.stack_pointer])
    }

    pub fn pop_double(&mut self) -> Option<u64>
    {
        // Get the lower and upper half.
        // The cast from u32 to u64 cannot fail.
        let lower: u64 = self.pop_single()? as u64;
        let upper: u64 = self.pop_single()? as u64;

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
        let lower: u32 = (value & Self::LOWER_MASK)
            .try_into()
            .expect("Failed to convert lower to u32");
        let upper: u32 = ((value & Self::UPPER_MASK) >> 32)
            .try_into()
            .expect("Failed to convert upper to u32");

        self.locals[index] = lower;
        self.locals[index + 1] = upper;
    }
}
