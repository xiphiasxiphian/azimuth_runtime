// Stack size is set at initiation and is hard coded somewhere.
// Theoretically this could become a config value at some point in the future

pub type StackEntry = u64;

#[derive(Debug)]
pub struct Stack
{
    // The entire data for the stack. This is just a static vector initially set
    // to a specific capacity
    stack: Vec<StackEntry>,
}

impl Stack
{
    /// Represents the size of one entry in the stack.
    ///
    /// Azimuth uses 64-bit values for each stack entry.
    /// This is largely because Azimuth is built for 64-bit
    /// computers, which want to be handling 64-bit values anyway.
    /// There is a world where using smaller 32-bit entries is still better
    /// owing to wasting less memory when representing values less than 64-bit,
    /// but it was decided that the problems with this outweigh the advantages.
    ///
    /// Some of the most common types used in common code are 32-bit ints,
    /// 64-bit ints and pointers, while bools are relatively uncommon and
    /// characters, while traditionally 8-bit, can end up being bigger when
    /// working with Unicode.
    ///
    /// Using 64-bit means that there aren't wasted clock cycles on having
    /// to stitch 64-bit values back together when stored on a 32-bit stack.
    pub const ENTRY_SIZE: usize = size_of::<StackEntry>();

    pub fn new(capacity: usize) -> Self
    {
        Stack {
            stack: vec![0; capacity],
        }
    }

    /// Creates the initial base stack frame based on the given locals and stack size.
    ///
    /// ### Warning
    /// If the given inputs cannot be used to create a stack frame that fits within the stack, then
    /// the operation will fail.
    pub fn initial_frame(&mut self, locals_size: usize, stack_size: usize) -> Option<StackFrame<'_>>
    {
        (locals_size + stack_size <= self.stack.len())
            .then(|| StackFrame::new(self, 0, locals_size, locals_size + stack_size))
    }
}

/// A frame within the stack.
///
/// This can be thought of as representing a specific region of memory within the stack,
/// defined as the total size of both the "stack" component, and the locals component.
/// The "stack" here represents the operand stack, used by the program to perform
/// operations such as arithmetic. The "locals" component is where local variables are stored.
/// The size of both these components are defined within the bytecode and are thus provided
/// by the compiler.
///
/// ## Example
/// ```
///     entry.push(1); // Add 1 onto the stack
///     assert_eq!(entry.pop(), Some(1)); // The variable on top of the stack is 1
///
///     entry.set_local(0, 1); // Set local variable 0 to 1
///     assert_eq!(entry.get_local(0), Some(1));
///
///     entry.with_next_frame(|x| {
///         entry.push(1);
///         assert_eq(entry.peek(), Some(1));
///     })
/// ```
#[derive(Debug)]
pub struct StackFrame<'a>
{
    origin: &'a mut Stack, //
    locals_base: usize,
    stack_base: usize,
    stack_pointer: usize,
    size: usize,
}

impl<'a> StackFrame<'a>
{
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


    /// Runs the given function within the context of the "next" stack frame.
    ///
    /// This functions creates a new stack frame on top of the current one, and will then run
    /// the given `action` within the context of that stack frame. This can mainly be used
    /// when functions are called to create its new stack frame and run it.
    ///
    /// ### Warning
    /// If the provided inputs cannot be used to create a valid stack frame (because of overflow)
    /// then this operation will fail. While the failure will be safe (see return value), it is
    /// worth saying that rarely will the execution of the program overall be able to continue from
    /// this.
    pub fn with_next_frame<F>(&'a mut self, locals_size: usize, stack_size: usize, action: F) -> bool
    where
        F: FnOnce(StackFrame<'a>),
    {
        (self.size + locals_size + stack_size <= self.origin.stack.len()) // Check if the new frame fits
            .then(|| {
                // Create the new frame and run the action given it.
                action(StackFrame::new(
                    self.origin,
                    self.size,
                    self.size + locals_size,
                    locals_size + stack_size,
                ));
            })
            .is_some() // If the creation failed, return false, otherwise return true.
    }

    /* As a general rule, all the stack operations are in some way "well defined".
     * This means that at all times these functions will fail safe, and will do something
     * expected whenever bad inputs are given, or they are run under "bad" circumstances
     *
     * In practice, this means that a "Stack Overflow" for the stack component, or an
     * "Index out of Bounds" for the locals component, the respective function will
     * refuse to perform the operation and instead return a value indicating this
     * failure. These failures can then theorectically be handled however at the
     * call site, but in general these errors are rarely recoverable.
     */

    /// Push value onto the stack.
    ///
    /// ### Possibles Errors
    /// Stack Overflow - returns `false`
    pub fn push(&mut self, value: StackEntry) -> bool
    {
        // Stack Overflow check
        if self.stack_pointer > self.size { return false; }

        self.origin.stack[self.stack_base + self.stack_pointer] = value;
        self.stack_pointer += 1;
        true
    }

    /// Pops a value of the stack, returning its value. If the value doesn't
    /// exist, return `None`.
    ///
    /// ### Possible Errors
    /// Empty Stack - return `None`
    pub fn pop(&mut self) -> Option<StackEntry>
    {
        (self.stack_pointer > 0).then(|| {
            self.stack_pointer -= 1;
            self.origin.stack[self.stack_base + self.stack_pointer]
        })
    }

    /// Peeks at the element on the top of the stack without removing it,
    /// or taking ownership of it.
    ///
    /// ### Possible Errors
    /// Empty Stack - return `None`
    pub fn peek(&self) -> Option<&StackEntry>
    {
        (self.stack_pointer > 0).then(|| &self.origin.stack[self.stack_base + self.stack_pointer])
    }

    /// Get the value of a local variable at the given index.
    ///
    /// ### Possible Errors
    /// Index out of Bounds - return `None`
    pub fn get_local(&self, index: usize) -> Option<StackEntry>
    {
        let idx = self.locals_base + index;
        (idx < self.stack_base + self.size).then(|| {
            self.origin.stack[idx]
        })
    }

    /// Set the value of a local variable at the given index, returning the previous
    /// value at that position.
    ///
    /// ### Possible Errors
    /// Index out of Bounds - return `None`
    pub fn set_local(&mut self, index: usize, value: StackEntry) -> Option<StackEntry>
    {
        let idx = self.locals_base + index; // Calculate the index based on the offset from the local base
        (idx < self.stack_base + self.size).then(|| {
            let prev = self.origin.stack[idx]; // Store previous value to return
            self.origin.stack[idx] = value;

            prev
        })
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

        frame.push(10);
        frame.push(20);

        assert_eq!(frame.pop().unwrap(), 20);
        assert_eq!(frame.pop().unwrap(), 10);
        assert!(frame.pop().is_none());
    }

    #[test]
    fn stack_frame_doubles()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push(1 << 33);

        assert_eq!(frame.pop().unwrap(), 1 << 33);
        assert!(frame.pop().is_none());
    }

    #[test]
    fn stack_frame_locals()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.set_local(0, 10);
        frame.set_local(1, 1 << 33);

        assert_eq!(frame.get_local(0), Some(10));
        assert_eq!(frame.get_local(1), Some(1 << 33));
    }
}
