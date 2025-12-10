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
    ISub,            // i.sub: Subtract top 2 values on the stack as integers. [value1], [value2] -> [result]
    F4Sub,           // f4.sub: Subtract top 2 values on the stack as float32. [value1], [value2] -> [result]
    F8Sub,           // f8.sub: Subtract top 2 values on the stack as float64. [value1], [value2] -> [result]
    IMul,            // i.mul: Multiply top 2 values on the stack as integers. [value1], [value2] -> [result]
    F4Mul,           // f4.mul: Multiply top 2 values on the stack as float32. [value1], [value2] -> [result]
    F8Mul,           // f8.mul: Multiply top 2 values on the stack as float64. [value1], [value2] -> [result]
    IDiv,            // i.div: Divide top 2 values on the stack as integers. [value1], [value2] -> [result]
    F4Div,           // f4.div: Divide top 2 values on the stack as float32. [value1], [value2] -> [result]
    F8Div,           // f8.div: Divide top 2 values on the stack as float64. [value1], [value2] -> [result]
    IRem,            // i.rem: Find remainder of division of top 2 values on the stack as integers. [value1], [value2] -> [result]
    F4Rem,           // f4.rem: Find remainder of division of top 2 values on the stack as float32. [value1], [value2] -> [result]
    F8Rem,           // f8.rem: Find remainder of division of top 2 values on the stack as float64. [value1], [value2] -> [result]
    INeg,            // i.neg: Negate top value on the stack as integer. [value] -> [result]
    F4Neg,           // f4.neg: Negate top value on the stack as float32. [value] -> [result]
    F8Neg,           // f8.neg: Negate top value on the stack as float64. [value] -> [result]
    Shl,             // shl: Logical Shift left of value top of the stack. [value1], [value2] -> [result]
    Shr,             // shr: Logical Shift Right of value top of the stack. [value1], [value2] -> [result]
    AShr,            // ashr: Arithmetic Shift Right of value top of the stack. [value1], [value2] -> [result]
    And,             // and: And operation on top 2 values on the stack. [value1], [value2] -> [result]
    Or,              // or: Or operation on top 2 values on the stack. [value1], [value2] -> [result]
    Xor,             // xor: Xor operation on top 2 values on the stack. [value1], [value2] -> [result]
    Not,             // not: Not operation on top value of the stack. [value] -> [result]
    Directive = 254, // .X: Directives for supplying metadata
    Unimplemented = 255,
}
