#[derive(Clone, Copy)]
pub enum Opcode
{
    Nop,             // nop: Do nothing. [No Change]
    IConst0,         // i.const.0: Push 0_i64 onto the stack. -> 0
    IConst1,         // i.const.1: Push 1_i64 onto the stack. -> 1
    IConst2,         // i.const.2: Push 2_i64 onto the stack. -> 2
    IConst3,         // i.const.3: Push 3_i64 onto the stack. -> 3
    F4Const0,        // f4.const.0: Push 0.0f onto the stack. -> 0.0f
    F4Const1,        // f4.const.1: Push 1.0f onto the stack. -> 1.0f
    F8Const0,        // f8.const.0: Push 0.0 onto the stack. -> 0.0
    F8Const1,        // f8.const.1: Push 1.0 onto the stack. -> 1.0
    IConst,          // i.const: Push a given 1 byte onto the stack -> [byte]
    IConstW,         // i.const.w: Push a given 2 bytes onto the stack. -> [byte2 << 8 | byte1]
    Const,           // const: Push the constant at the given index onto the stack. -> [constant]
    LdArg0,          // ld.arg.0: Load the local variable at index 0 onto the stack. -> [local0]
    LdArg1,          // ld.arg.1: Load the local variable at index 1 onto the stack. -> [local1]
    LdArg2,          // ld.arg.2: Load the local variable at index 2 onto the stack. -> [local2]
    LdArg3,          // ld.arg.3: Load the local variable at index 3 onto the stack. -> [local3]
    LdArg,           // ld.arg: Load local variable to the stack. -> [local{index}]
    StArg0,          // st.arg.0: Store top of the stack into local variable 0. [value] ->
    StArg1,          // st.arg.1: Store top of the stack into local variable 1. [value] ->
    StArg2,          // st.arg.2: Store top of the stack into local variable 2. [value] ->
    StArg3,          // st.arg.3: Store top of the stack into local variable 3. [value] ->
    StArg,           // st.arg: Store top of the stack into local variable. [value] ->
    Pop,             // pop: Discard the top of the stack. [value] ->
    Dup,             // dup: Duplicate the value on the top of the stack. [value] -> [value], [value]
    Swap,            // swap: Swap the top 2 stack entries. [value1], [value2] -> [value2], [value1]
    Ret,             // ret: Return out of the current function. -> !
    RetVal,          // ret.val: Return with the value top of the stack. [value] -> !
    IAdd,            // i.add: Add top 2 values on the stack as integers. [value1], [value2] -> [result]
    F4Add,           // f4.add: Add top 2 values on the stack as float32. [value1], [value2] -> [result]
    F8Add,           // f8.add: Add top 2 values on the stack as float64. [value1], [value2] -> [result]
    Directive = 254, // .X: Directives for supplying metadata
    Unimplemented = 255,
}
