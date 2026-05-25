use std::slice::SliceIndex;

use crate::{
    engine::{RunnerError, opcode_handler::ExecutionError},
    guard,
    memory::stack::entry::StackEntry,
};

pub mod convert;
pub mod entry;
pub mod stackable;

// Stack size is set at initiation and is hard coded somewhere.
// Theoretically this could become a config value at some point in the future

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
    pub const ENTRY_SIZE: usize = size_of::<u64>();

    pub fn new(capacity: usize) -> Self
    {
        Stack {
            stack: vec![StackEntry::Unsigned(0); capacity],
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

    pub fn iter(&self) -> impl Iterator<Item = &StackEntry>
    {
        self.stack.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut StackEntry>
    {
        self.stack.iter_mut()
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
/// ## Memory layout
///
/// ```text
///   ┌──────────────────────────────────────────────────────────────────┐
///   │ locals[0..locals_size]        │ operand stack[0..stack_size]     │
///   └──────────────────────────────────────────────────────────────────┘
///   ↑ locals_base                  ↑ stack_base
/// ```
///
/// When `with_next_frame` is called with `param_count > 0`, the last
/// `param_count` entries that the *caller* pushed onto its operand stack
/// overlap with the *callee*'s first `param_count` locals.  No data is
/// copied; the callee sees the arguments already in place.
///
/// ```text
///   caller frame
///   ┌──────────────────┬──────────────────────┐
///   │ caller locals    │ … │ arg0 │ arg1 │    │
///   └──────────────────┴──────────────────────┘
///                           ↑ callee locals_base
///                      callee frame
///                      ┌───────────────────────────┬─────────────────┐
///                      │ arg0 │ arg1 │ extra local │ callee op stack │
///                      └───────────────────────────┴─────────────────┘
/// ```
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
    /// The last `param_count` entries currently on top of *this* frame's
    /// operand stack become the first `param_count` locals of the new frame
    /// (zero-copy overlap).  After `action` returns the caller's
    /// `stack_pointer` is decremented by `param_count`, consuming the
    /// arguments.
    ///
    /// ### Warning
    /// If the provided inputs cannot be used to create a valid stack frame (because of overflow)
    /// then this operation will fail. While the failure will be safe (see return value), it is
    /// worth noting that rarely will the execution of the program overall be able to continue from
    /// this.
    pub fn with_next_frame<'b, F>(
        &'b mut self,
        locals_size: usize,
        stack_size: usize,
        param_count: usize,
        action: F,
    ) -> Result<Option<StackEntry>, RunnerError>
    where
        F: FnOnce(StackFrame<'b>) -> Result<Option<StackEntry>, RunnerError>,
    {
        guard!(
            param_count <= self.stack_pointer,
            RunnerError::ExecutionError(ExecutionError::MissingParams)
        );

        // The parameters are the last `param_count` items on the operand stack.
        // They become the first `param_count` locals of the new frame.
        let current_top = self.stack_base + self.stack_pointer;
        let new_locals_base = current_top - param_count;

        let new_stack_base = new_locals_base + locals_size;
        let total_required_capacity = locals_size + stack_size;

        // Bounds check against the physical stack limit.
        if new_stack_base + stack_size > self.origin.stack.len()
        {
            return Err(RunnerError::StackOverflow);
        }

        // Create the new frame.  Its first locals already contain the
        // arguments, which are physically sitting on the caller's operand
        // stack region.
        let new_frame = StackFrame::new(self.origin, new_locals_base, new_stack_base, total_required_capacity);

        let result = action(new_frame)?;

        // Consume the arguments from the caller's operand stack.
        self.stack_pointer -= param_count;

        Ok(result)
    }

    /* As a general rule, all the stack operations are in some way "well defined".
     * This means that at all times these functions will fail safe, and will do something
     * expected whenever bad inputs are given, or they are run under "bad" circumstances
     *
     * In practice, this means that a "Stack Overflow" for the stack component, or an
     * "Index out of Bounds" for the locals component, the respective function will
     * refuse to perform the operation and instead return a value indicating this
     * failure. These failures can then theoretically be handled however at the
     * call site, but in general these errors are rarely recoverable.
     */

    /// Push value onto the stack.
    ///
    /// ### Possible Errors
    /// Stack Overflow - returns `false`
    pub fn push(&mut self, value: StackEntry) -> bool
    {
        // Stack Overflow check
        if self.stack_pointer > self.size
        {
            return false;
        }

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
        (self.stack_pointer > 0).then(|| &self.origin.stack[self.stack_base + self.stack_pointer - 1])
    }

    /// Get the value of a local variable at the given index.
    ///
    /// ### Possible Errors
    /// Index out of Bounds - return `None`
    pub fn get_local<I>(&self, index: I) -> Option<&I::Output>
    where
        I: SliceIndex<[StackEntry]>,
    {
        // BUG FIX: was `self.stack_base + self.size`, which overestimates the
        // upper bound by `locals_size` entries (stack_base already includes the
        // locals offset).  The correct ceiling is `locals_base + size`.
        let limit = self.locals_base + self.size;
        self.origin.stack.get(self.locals_base..limit)?.get(index)
    }

    /// Set the value of a local variable at the given index, returning the previous
    /// value at that position.
    ///
    /// ### Possible Errors
    /// Index out of Bounds - return `None`
    pub fn set_local(&mut self, index: usize, value: StackEntry) -> Option<StackEntry>
    {
        let idx = self.locals_base + index;
        // BUG FIX: same ceiling correction as get_local.
        (idx < self.locals_base + self.size).then(|| {
            let prev = self.origin.stack[idx];
            self.origin.stack[idx] = value;
            prev
        })
    }
}

#[cfg(test)]
mod stack_tests
{
    use super::*;

    // ── Stack ─────────────────────────────────────────────────────────────────

    #[test]
    fn stack_init_works()
    {
        let stack: Stack = Stack::new(1024);
        assert_eq!(stack.stack.len(), 1024);
    }

    #[test]
    fn stack_zero_capacity()
    {
        let stack = Stack::new(0);
        assert_eq!(stack.stack.len(), 0);
        // Neither of these should be constructable.
        // initial_frame(0, 0) requires 0 <= 0, which is true,
        // so it *can* be created; any subsequent frame would overflow.
    }

    // ── StackFrame construction ───────────────────────────────────────────────

    #[test]
    fn new_stack_frame_correct_info()
    {
        let mut stack: Stack = Stack::new(1024);
        let frame = stack.initial_frame(4, 4).unwrap();

        assert_eq!(frame.locals_base, 0);
        assert_eq!(frame.stack_base, 4);
        assert_eq!(frame.stack_pointer, 0);
        assert_eq!(frame.size, 8); // locals_size + stack_size
    }

    #[test]
    fn initial_frame_exact_capacity_succeeds()
    {
        let mut stack = Stack::new(8);
        assert!(stack.initial_frame(4, 4).is_some());
    }

    #[test]
    fn initial_frame_exceeds_capacity_fails()
    {
        let mut stack = Stack::new(1024);
        assert!(stack.initial_frame(513, 513).is_none());
    }

    // ── push / pop / peek ─────────────────────────────────────────────────────

    #[test]
    fn push_pop_lifo_order()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push(10_u64.into());
        frame.push(20_u64.into());

        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(20));
        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(10));
        assert!(frame.pop().is_none());
    }

    #[test]
    fn pop_empty_stack_returns_none()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();
        assert!(frame.pop().is_none());
    }

    #[test]
    fn push_64bit_value_roundtrips()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();
        let large = StackEntry::Unsigned(1u64 << 33);
        frame.push(large);
        assert_eq!(frame.pop().unwrap(), large);
    }

    #[test]
    fn peek_returns_top_without_removing()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        // Stack is empty.
        assert!(frame.peek().is_none());

        frame.push(StackEntry::Unsigned(42));
        assert_eq!(frame.peek(), Some(&StackEntry::Unsigned(42)));

        // peek must not advance the pointer: pop still returns 42.
        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(42));

        // Stack is empty again.
        assert!(frame.peek().is_none());
    }

    #[test]
    fn peek_reflects_most_recently_pushed_item()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(0, 8).unwrap();

        frame.push(StackEntry::Unsigned(1));
        assert_eq!(frame.peek(), Some(&StackEntry::Unsigned(1)));

        frame.push(StackEntry::Unsigned(2));
        assert_eq!(frame.peek(), Some(&StackEntry::Unsigned(2)));

        frame.pop();
        assert_eq!(frame.peek(), Some(&StackEntry::Unsigned(1)));
    }

    // ── get_local / set_local ─────────────────────────────────────────────────

    #[test]
    fn locals_read_write()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.set_local(0, 10_u64.into());
        frame.set_local(1, StackEntry::Unsigned(1u64 << 33));

        assert_eq!(frame.get_local(0), Some(&StackEntry::Unsigned(10)));
        assert_eq!(frame.get_local(1), Some(&StackEntry::Unsigned(1 << 33)));
    }

    #[test]
    fn set_local_returns_previous_value()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.set_local(0, StackEntry::Unsigned(1));
        let prev = frame.set_local(0, StackEntry::Unsigned(2));

        assert_eq!(prev, Some(StackEntry::Unsigned(1)));
        assert_eq!(frame.get_local(0), Some(&StackEntry::Unsigned(2)));
    }

    #[test]
    fn get_local_out_of_bounds_returns_none()
    {
        let mut stack = Stack::new(1024);
        let frame = stack.initial_frame(4, 4).unwrap();
        // locals_size = 4, stack_size = 4; total addressable = 8 entries.
        // Index 8 is one past the end.
        assert!(frame.get_local(8).is_none());
    }

    #[test]
    fn set_local_out_of_bounds_returns_none()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();
        assert!(frame.set_local(8, StackEntry::Unsigned(99)).is_none());
    }

    // ── with_next_frame — basic nesting ───────────────────────────────────────

    /// With no params and an empty operand stack the new frame's locals start
    /// immediately where the caller's operand stack begins (they share the same
    /// physical address, just interpreted differently).
    ///
    /// Before the fix the test expected `locals_base=8, stack_base=12`, which
    /// matched the *old* non-overlapping layout where every frame was appended
    /// after the previous one's full `size`.  With overlap the new frame starts
    /// at `stack_base + stack_pointer = 4 + 0 = 4`.
    #[test]
    fn stack_frame_nesting_layout()
    {
        let mut stack: Stack = Stack::new(1024);
        let mut frame1 = stack.initial_frame(4, 4).unwrap();
        // frame1: locals_base=0, stack_base=4, stack_pointer=0

        assert!(
            frame1
                .with_next_frame(4, 4, 0, |f| {
                    // new_locals_base = stack_base + stack_pointer - param_count
                    //                 = 4 + 0 - 0 = 4
                    // new_stack_base  = 4 + locals_size = 4 + 4 = 8
                    assert_eq!(f.locals_base, 4);
                    assert_eq!(f.stack_base, 8);
                    assert_eq!(f.stack_pointer, 0);
                    Ok(None)
                })
                .is_ok()
        );
    }

    #[test]
    fn with_next_frame_return_value_propagates()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(0, 8).unwrap();

        let result = frame
            .with_next_frame(0, 4, 0, |_| Ok(Some(StackEntry::Unsigned(0xDEAD_BEEF))))
            .unwrap();

        assert_eq!(result, Some(StackEntry::Unsigned(0xDEAD_BEEF)));
    }

    #[test]
    fn with_next_frame_none_return_value_propagates()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(0, 8).unwrap();
        let result = frame.with_next_frame(0, 4, 0, |_| Ok(None)).unwrap();
        assert!(result.is_none());
    }

    // ── with_next_frame — overflow ────────────────────────────────────────────

    /// Before the fix the test used `with_next_frame(20, 20, 0)` which, with
    /// the new overlap layout, starts at `stack_base + stack_pointer = 512`
    /// and only needs `512 + 20 + 20 = 552 ≤ 1024` — not an overflow.
    /// The inner frame must be large enough that `new_stack_base + stack_size`
    /// exceeds 1024; here `512 + 300 + 300 = 1112 > 1024`.
    #[test]
    fn stack_overflow_detected()
    {
        let mut stack: Stack = Stack::new(1024);

        // A frame requiring more than the total capacity must be rejected.
        assert!(stack.initial_frame(513, 513).is_none());

        // A frame that fills the stack exactly is fine…
        let mut frame = stack.initial_frame(512, 512).unwrap();
        // …but a sub-frame that would extend beyond the end must fail.
        assert!(frame.with_next_frame(300, 300, 0, |_| Ok(None)).is_err());
    }

    #[test]
    fn stack_overflow_when_pushed_items_displace_inner_frame()
    {
        // Use a small stack to make the arithmetic easy to follow.
        //   capacity = 24
        //   outer: locals_base=0, stack_base=8, size=16 (locals=8, stack=8)
        let mut stack = Stack::new(24);
        let mut outer = stack.initial_frame(8, 8).unwrap();

        // Push 6 items; stack_pointer is now 6.
        // current_top = 8 + 6 = 14
        for i in 0..6u64
        {
            outer.push(StackEntry::Unsigned(i));
        }

        // with_next_frame(6, 6, 0):
        //   new_locals_base = 14
        //   new_stack_base  = 14 + 6 = 20
        //   check: 20 + 6 = 26 > 24  →  StackOverflow
        assert!(outer.with_next_frame(6, 6, 0, |_| Ok(None)).is_err());
    }

    // ── with_next_frame — parameter overlap ───────────────────────────────────

    /// The last `param_count` entries on the caller's operand stack must be
    /// visible as the first `param_count` locals inside the callee — with no
    /// data copy: they share the same physical slots.
    #[test]
    fn params_overlap_with_callee_locals()
    {
        let mut stack = Stack::new(1024);
        // outer: locals_base=0, stack_base=4, size=12 (locals=4, stack=8)
        let mut outer = stack.initial_frame(4, 8).unwrap();

        outer.push(StackEntry::Unsigned(111));
        outer.push(StackEntry::Unsigned(222));
        // stack_pointer = 2; current_top = 4 + 2 = 6

        outer
            .with_next_frame(4, 4, 2, |inner| {
                // new_locals_base = 6 - 2 = 4
                // local[0] ← stack[4] = 111
                // local[1] ← stack[5] = 222
                assert_eq!(inner.locals_base, 4);
                assert_eq!(inner.get_local(0), Some(&StackEntry::Unsigned(111)));
                assert_eq!(inner.get_local(1), Some(&StackEntry::Unsigned(222)));
                Ok(None)
            })
            .unwrap();
    }

    /// After `with_next_frame` returns the caller's `stack_pointer` must be
    /// decremented by `param_count`, consuming the arguments.
    #[test]
    fn caller_stack_pointer_decremented_after_call()
    {
        let mut stack = Stack::new(1024);
        let mut outer = stack.initial_frame(0, 8).unwrap();

        outer.push(StackEntry::Unsigned(1));
        outer.push(StackEntry::Unsigned(2));
        outer.push(StackEntry::Unsigned(3));
        assert_eq!(outer.stack_pointer, 3);

        outer.with_next_frame(3, 4, 3, |_| Ok(None)).unwrap();

        // All three arguments were consumed.
        assert_eq!(outer.stack_pointer, 0);
        assert!(outer.pop().is_none());
    }

    #[test]
    fn caller_retains_non_param_entries_after_call()
    {
        let mut stack = Stack::new(1024);
        let mut outer = stack.initial_frame(0, 8).unwrap();

        // Push 3 items; only the top 2 are params.
        outer.push(StackEntry::Unsigned(10));
        outer.push(StackEntry::Unsigned(20));
        outer.push(StackEntry::Unsigned(30));

        outer.with_next_frame(2, 4, 2, |_| Ok(None)).unwrap();

        // stack_pointer should be back to 1 (the non-param entry remains).
        assert_eq!(outer.stack_pointer, 1);
        assert_eq!(outer.pop(), Some(StackEntry::Unsigned(10)));
    }

    /// Extra locals beyond `param_count` are zero-initialised from the
    /// pre-zeroed backing Vec and are independently writable by the callee.
    #[test]
    fn callee_extra_locals_are_independent()
    {
        let mut stack = Stack::new(1024);
        let mut outer = stack.initial_frame(0, 8).unwrap();

        outer.push(StackEntry::Unsigned(42));

        outer
            .with_next_frame(3, 4, 1, |mut inner| {
                // local[0] = param = 42
                assert_eq!(inner.get_local(0), Some(&StackEntry::Unsigned(42)));
                // local[1] and local[2] are extra locals — write to them.
                inner.set_local(1, StackEntry::Unsigned(100));
                inner.set_local(2, StackEntry::Unsigned(200));
                assert_eq!(inner.get_local(1), Some(&StackEntry::Unsigned(100)));
                assert_eq!(inner.get_local(2), Some(&StackEntry::Unsigned(200)));
                Ok(None)
            })
            .unwrap();
    }

    /// Because the param slots are *shared* memory, a write by the callee to
    /// one of its first `param_count` locals is immediately visible in the
    /// backing store (though the caller will not normally access those slots
    /// via get_local after the call).
    #[test]
    fn callee_write_to_param_local_is_visible_in_backing_store()
    {
        let mut stack = Stack::new(1024);
        let mut outer = stack.initial_frame(0, 8).unwrap();

        outer.push(StackEntry::Unsigned(1));
        outer.push(StackEntry::Unsigned(2));

        outer
            .with_next_frame(2, 4, 2, |mut inner| {
                // Overwrite the shared param slots.
                inner.set_local(0, StackEntry::Unsigned(99));
                inner.set_local(1, StackEntry::Unsigned(100));
                Ok(None)
            })
            .unwrap();

        // The original push values live in the backing store; the callee
        // overwrote them.  Verify via the raw backing Vec through iter().
        let vals: Vec<StackEntry> = stack.iter().take(4).cloned().collect();
        // slot 0 = local[0] of outer = callee's local[0] = 99
        assert_eq!(vals[0], StackEntry::Unsigned(99));
        // slot 1 = local[1] of outer = callee's local[1] = 100
        assert_eq!(vals[1], StackEntry::Unsigned(100));
    }

    // ── with_next_frame — MissingParams ──────────────────────────────────────

    /// Requesting more params than items on the operand stack must fail, even
    /// when `stack_base` is large enough that the old `current_top.checked_sub`
    /// guard would have silently succeeded.
    #[test]
    fn missing_params_error_when_stack_is_empty()
    {
        let mut stack = Stack::new(1024);
        // locals_size=8 means stack_base=8; the old checked_sub on current_top
        // (= 8 + 0 = 8) would have returned Some(6) for param_count=2,
        // incorrectly harvesting params from the locals region.
        let mut frame = stack.initial_frame(8, 8).unwrap();

        let result = frame.with_next_frame(4, 4, 2, |_| Ok(None));
        assert!(matches!(
            result,
            Err(RunnerError::ExecutionError(ExecutionError::MissingParams))
        ));
    }

    #[test]
    fn missing_params_error_when_fewer_items_than_requested()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(0, 8).unwrap();
        frame.push(StackEntry::Unsigned(1)); // only 1 item pushed

        let result = frame.with_next_frame(4, 4, 3, |_| Ok(None)); // wants 3
        assert!(matches!(
            result,
            Err(RunnerError::ExecutionError(ExecutionError::MissingParams))
        ));
    }

    #[test]
    fn zero_params_always_succeeds_regardless_of_stack_depth()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(0, 8).unwrap();
        // Nothing pushed; param_count=0 must never trigger MissingParams.
        assert!(frame.with_next_frame(4, 4, 0, |_| Ok(None)).is_ok());
    }

    // ── with_next_frame — deep nesting with overlap ───────────────────────────

    #[test]
    fn three_level_nesting_layouts_correctly()
    {
        //  frame1: locals=0, stack_base=4, sp=0
        let mut stack = Stack::new(1024);
        let mut frame1 = stack.initial_frame(4, 8).unwrap();

        frame1.push(StackEntry::Unsigned(10)); // will be param for frame2
        // frame1.sp = 1; current_top = 4 + 1 = 5

        frame1
            .with_next_frame(2, 8, 1, |mut frame2| {
                // frame2: new_locals_base = 5 - 1 = 4
                //         new_stack_base  = 4 + 2 = 6
                assert_eq!(frame2.locals_base, 4);
                assert_eq!(frame2.stack_base, 6);
                assert_eq!(frame2.get_local(0), Some(&StackEntry::Unsigned(10)));

                frame2.push(StackEntry::Unsigned(20)); // param for frame3
                // frame2.sp = 1; current_top = 6 + 1 = 7

                frame2.with_next_frame(1, 4, 1, |frame3| {
                    // frame3: new_locals_base = 7 - 1 = 6
                    //         new_stack_base  = 6 + 1 = 7
                    assert_eq!(frame3.locals_base, 6);
                    assert_eq!(frame3.stack_base, 7);
                    assert_eq!(frame3.get_local(0), Some(&StackEntry::Unsigned(20)));
                    Ok(None)
                })
            })
            .unwrap();
    }

    // ── Legacy regression tests (kept for coverage) ───────────────────────────

    #[test]
    fn stack_frame_singles()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push(10_u64.into());
        frame.push(20_u64.into());

        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(20));
        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(10));
        assert!(frame.pop().is_none());
    }

    #[test]
    fn stack_frame_doubles()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.push(StackEntry::Unsigned(1 << 33));

        assert_eq!(frame.pop().unwrap(), StackEntry::Unsigned(1 << 33));
        assert!(frame.pop().is_none());
    }

    #[test]
    fn stack_frame_locals()
    {
        let mut stack = Stack::new(1024);
        let mut frame = stack.initial_frame(4, 4).unwrap();

        frame.set_local(0, 10_u64.into());
        frame.set_local(1, StackEntry::from((1 as u64) << 33));

        assert_eq!(frame.get_local(0), Some(&StackEntry::Unsigned(10)));
        assert_eq!(frame.get_local(1), Some(&StackEntry::Unsigned(1 << 33)));
    }
}
