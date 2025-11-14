// Stack size is set at initiation and is hard coded somewhere.
// Theoretically this could become a config value at some point in the future
#[derive(Debug)]
pub struct Stack
{
    // The entire data for the stack. This is just a static vector initially set
    // to a specific capacity
    stack: Vec<u32>,
}

impl Stack
{
    pub fn new(capacity: usize) -> Self
    {
        Stack {
            stack: vec![0; capacity],
        }
    }

    pub fn initial_frame<'a>(
        &'a mut self,
        locals_size: usize,
        stack_size: usize,
    ) -> Option<StackFrame<'a>>
    {
        (locals_size + stack_size <= self.stack.len())
            .then(|| StackFrame::new(self, 0, locals_size, locals_size + stack_size))
    }
}

// At some point I might revisit this and make it all work slightly more inline.
// But for now this is a very basic implementation
#[derive(Debug)]
pub struct StackFrame<'a>
{
    origin: &'a mut Stack,
    locals_base: usize,
    stack_base: usize,
    stack_pointer: usize,
    size: usize,
}

impl<'a> StackFrame<'a>
{
    const UPPER_LOWER_OFFSET: u64 = 32;

    const LOWER_MASK: u64 = 0xFFFFFFFF;
    const UPPER_MASK: u64 = Self::LOWER_MASK << Self::UPPER_LOWER_OFFSET;

    pub fn new(origin: &'a mut Stack, locals_base: usize, stack_base: usize, size: usize) -> Self
    {
        StackFrame {
            origin,
            locals_base,
            stack_base,
            stack_pointer: 0,
            size,
        }
    }

    pub fn with_next_frame<F>(&'a mut self, locals_size: usize, stack_size: usize, f: F) -> bool
    where
        F: FnOnce(StackFrame<'a>),
    {
        (self.size + locals_size + stack_size <= self.origin.stack.len())
            .then(|| {
                f(StackFrame::new(
                    self.origin,
                    self.size,
                    self.size + locals_size,
                    locals_size + stack_size,
                ))
            })
            .is_some()
    }

    pub fn push_single(&mut self, value: u32)
    {
        self.origin.stack[self.stack_base + self.stack_pointer] = value;
        self.stack_pointer += 1;
    }

    pub fn push_double(&mut self, value: u64)
    {
        let (lower, upper) = Self::split_double(value);

        // The upper half is stored first in the stack compared with the lower half.
        // This means that the first thing popped off the stack will be the lower half
        self.push_single(upper);
        self.push_single(lower);
    }

    pub fn pop_single(&mut self) -> Option<u32>
    {
        (self.stack_pointer > 0).then(|| {
            self.stack_pointer -= 1;
            self.origin.stack[self.stack_base + self.stack_pointer]
        })
    }

    pub fn pop_double(&mut self) -> Option<u64>
    {
        let lower = self.pop_single()?;
        let upper = self.pop_single()?;

        Some(Self::combine_double(lower, upper))
    }

    pub fn get_local_single(&self, index: usize) -> u32
    {
        self.origin.stack[self.locals_base + index]
    }

    pub fn get_local_double(&self, index: usize) -> u64
    {
        Self::combine_double(
            self.get_local_single(index),
            self.get_local_single(index + 1)
        )
    }

    pub fn set_local_single(&mut self, index: usize, value: u32)
    {
        self.origin.stack[self.locals_base + index] = value;
    }

    pub fn set_local_double(&mut self, index: usize, value: u64)
    {
        let (lower, upper) = Self::split_double(value);

        self.set_local_single(index, lower);
        self.set_local_single(index + 1, upper);
    }

    fn split_double(value: u64) -> (u32, u32)
    {
        let lower: u32 = (value & Self::LOWER_MASK)
            .try_into()
            .expect("Failed to convert lower to u32");
        let upper: u32 = ((value & Self::UPPER_MASK) >> Self::UPPER_LOWER_OFFSET)
            .try_into()
            .expect("Failed to convert upper to u32");

        (lower, upper)
    }

    fn combine_double(lower: u32, upper: u32) -> u64
    {
        let low = lower as u64;
        let high = upper as u64;

        (high << Self::UPPER_LOWER_OFFSET) | low
    }
}

#[cfg(test)]
mod stack_tests
{
    use super::*;

    #[test]
    fn stack_init_works()
    {
        let stack: Stack = Stack::new(1024);
        assert_eq!(stack.stack.len(), 1024);
    }

    #[test]
    fn new_stack_frame_correct_info()
    {
        let mut stack: Stack = Stack::new(1024);
        let frame = stack.initial_frame(4, 4).unwrap();

        assert_eq!(frame.locals_base, 0);
        assert_eq!(frame.stack_base, 4);
        assert_eq!(frame.stack_pointer, 0);
    }

    #[test]
    fn stack_frame_nesting()
    {
        let mut stack: Stack = Stack::new(1024);
        let mut frame1 = stack.initial_frame(4, 4).unwrap();
        assert!(frame1.with_next_frame(4, 4, |f| {
            assert_eq!(f.locals_base, 8);
            assert_eq!(f.stack_base, 12);
            assert_eq!(f.stack_pointer, 0);
        }));
    }

    #[test]
    fn stack_overflow_detected()
    {
        let mut stack: Stack = Stack::new(1024);
        let frame1 = stack.initial_frame(513, 513);

        assert!(frame1.is_none());
        let mut frame2 = stack.initial_frame(512, 512).unwrap();

        assert!(!frame2.with_next_frame(20, 20, |_| {}));
    }

    #[test]
    fn stack_frame_singles()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push_single(10);
        frame.push_single(20);

        assert_eq!(frame.pop_single().unwrap(), 20);
        assert_eq!(frame.pop_single().unwrap(), 10);
        assert!(frame.pop_single().is_none());
    }

    #[test]
    fn stack_frame_doubles()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push_double(1 << 33);

        assert_eq!(frame.pop_double().unwrap(), 1 << 33);
        assert!(frame.pop_double().is_none());
    }

    #[test]
    fn stack_frame_locals()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.set_local_single(0, 10);
        frame.set_local_double(1, 1 << 33);

        assert_eq!(frame.get_local_single(0), 10);
        assert_eq!(frame.get_local_double(1), 1 << 33);
    }
}
