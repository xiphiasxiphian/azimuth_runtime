// The narrowing primitive conversion behaviour here is desired
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::min_ident_chars)]

use std::{
    cmp::Ordering, ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not, Rem, Shl, Shr, Sub}, ptr::NonNull
};

use crate::memory::stack::convert::StackableConvert;

#[derive(Debug, Clone, Copy)]
pub enum StackEntry
{
    Unsigned(u64),
    Signed(i64),
    Character(char),
    Float(f32),
    Double(f64),
    Reference(Option<NonNull<u8>>),
}

impl StackEntry
{
    pub fn try_binary_operation<I1, I2, O, F>(self, other: Self, op: F) -> Option<Self>
    where
        Self: TryInto<I1> + TryInto<I2>,
        O: Into<StackEntry>,
        F: Fn(I1, I2) -> O,
    {
        let first = self.try_into().ok()?;
        let second = other.try_into().ok()?;

        Some(op(first, second).into())
    }

    pub fn try_map<T1, T2, F>(self, op: F) -> Option<Self>
    where
        Self: TryInto<T1>,
        T2: Into<StackEntry>,
        F: Fn(T1) -> T2,
    {
        Some(op(self.try_into().ok()?).into())
    }

    pub fn cast<F, T>(self) -> Option<Self>
    where
        F: TryFrom<Self>,
        T: Into<Self> + StackableConvert<F>,
    {
        self.try_map(<T>::convert)
    }

    fn as_f64(&self) -> Option<f64>
    {
        match self
        {
            Self::Unsigned(n) => Some(*n as f64),
            Self::Signed(n) => Some(*n as f64),
            Self::Float(n) => Some(*n as f64),
            Self::Double(n) => Some(*n),
            _ => None,
        }
    }
}

impl PartialEq for StackEntry {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // Same-type comparisons
            (Self::Unsigned(a), Self::Unsigned(b)) => a == b,
            (Self::Signed(a), Self::Signed(b)) => a == b,
            (Self::Character(a), Self::Character(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Double(a), Self::Double(b)) => a == b,
            (Self::Reference(a), Self::Reference(b)) => a == b,

            // Cross-type integer comparisons
            (Self::Unsigned(a), Self::Signed(b)) => *b >= 0 && *a == (*b as u64),
            (Self::Signed(a), Self::Unsigned(b)) => *a >= 0 && (*a as u64) == *b,

            // Cross-type float/integer mixing fallback
            _ => match (self.as_f64(), other.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => false, // Non-numeric mismatched types (e.g., Character == Float) are false
            },
        }
    }
}

impl PartialOrd for StackEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            // Same-type comparisons
            (Self::Unsigned(a), Self::Unsigned(b)) => a.partial_cmp(b),
            (Self::Signed(a), Self::Signed(b)) => a.partial_cmp(b),
            (Self::Character(a), Self::Character(b)) => a.partial_cmp(b),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(b),
            (Self::Double(a), Self::Double(b)) => a.partial_cmp(b),
            (Self::Reference(a), Self::Reference(b)) => a.partial_cmp(b),

            // Cross-type integer comparisons
            (Self::Unsigned(a), Self::Signed(b)) if *b < 0 => Some(Ordering::Greater),
            (Self::Unsigned(a), Self::Signed(b)) => a.partial_cmp(&(*b as u64)),
            (Self::Signed(a), Self::Unsigned(b)) if *a < 0 => Some(Ordering::Less),
            (Self::Signed(a), Self::Unsigned(b)) => (*a as u64).partial_cmp(b),

            // Cross-type float/integer mixing fallback
            _ => match (self.as_f64(), other.as_f64()) {
                (Some(a), Some(b)) => a.partial_cmp(&b),
                _ => None, // Non-numeric mismatched types return None
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ARITHMETIC MACRO (Add, Sub, Mul, Div, Rem)
//    - Integers: uses `wrapping_<method>` (e.g., wrapping_add)
//    - Floats: uses standard operators (e.g., +)
// ---------------------------------------------------------------------------
macro_rules! impl_arithmetic {
    ($trait:ident, $fn:ident, $int_op:expr, $float_op:expr) => {
        impl $trait for StackEntry
        {
            type Output = Option<Self>;

            #[expect(clippy::redundant_closure_for_method_calls, reason = "Trait stuff")]
            fn $fn(self, other: Self) -> Self::Output
            {
                // 1. Try Unsigned (u64)
                self.try_binary_operation::<u64, u64, u64, _>(other, $int_op)
                    // 2. Try Signed (i64)
                    .or_else(|| self.try_binary_operation::<i64, i64, i64, _>(other, $int_op))
                    // 3. Try Float (f32)
                    .or_else(|| self.try_binary_operation::<f32, f32, f32, _>(other, $float_op))
                    // 4. Try Double (f64)
                    .or_else(|| self.try_binary_operation::<f64, f64, f64, _>(other, $float_op))
            }
        }
    };
}

// Implementations
impl_arithmetic!(Add, add, |a, b| a.wrapping_add(b), |a, b| a + b);
impl_arithmetic!(Sub, sub, |a, b| a.wrapping_sub(b), |a, b| a - b);
impl_arithmetic!(Mul, mul, |a, b| a.wrapping_mul(b), |a, b| a * b);
impl_arithmetic!(Div, div, |a, b| a.wrapping_div(b), |a, b| a / b);
impl_arithmetic!(Rem, rem, |a, b| a.wrapping_rem(b), |a, b| a % b);

// ---------------------------------------------------------------------------
// BITWISE MACRO (BitAnd, BitOr, BitXor)
//    - Integers Only. Floats return None.
// ---------------------------------------------------------------------------
macro_rules! impl_bitwise {
    ($trait:ident, $fn:ident, $op:expr) => {
        impl $trait for StackEntry
        {
            type Output = Option<Self>;

            fn $fn(self, other: Self) -> Self::Output
            {
                self.try_binary_operation::<u64, u64, u64, _>(other, $op)
                    .or_else(|| self.try_binary_operation::<i64, i64, i64, _>(other, $op))
            }
        }
    };
}

impl_bitwise!(BitAnd, bitand, |a, b| a & b);
impl_bitwise!(BitOr, bitor, |a, b| a | b);
impl_bitwise!(BitXor, bitxor, |a, b| a ^ b);

// ---------------------------------------------------------------------------
// SHIFT MACRO (Shl, Shr)
//    - Integers Only.
//    - Special Case: shifts require the RHS to be cast to u32.
// ---------------------------------------------------------------------------
macro_rules! impl_shift {
    ($trait:ident, $fn:ident, $method:ident) => {
        impl $trait for StackEntry
        {
            type Output = Option<Self>;

            fn $fn(self, other: Self) -> Self::Output
            {
                // Case 1: Unsigned << Unsigned
                self.try_binary_operation::<u64, u64, u64, _>(other, |a, b| a.$method(b as u32))
                    // Case 2: Signed << Signed
                    .or_else(|| self.try_binary_operation::<i64, i64, i64, _>(other, |a, b| a.$method(b as u32)))
                // Note: If you want to allow Shifting Signed by Unsigned, you would add more chains here.
            }
        }
    };
}

impl_shift!(Shl, shl, wrapping_shl);
impl_shift!(Shr, shr, wrapping_shr);

// ---------------------------------------------------------------------------
// UNARY MACRO (Not, Neg)
//    - Not (!): Integers only.
//    - Neg (-): Signed Ints (wrapping), Floats (standard).
// ---------------------------------------------------------------------------

impl Not for StackEntry
{
    type Output = Option<Self>;
    fn not(self) -> Self::Output
    {
        self.try_map::<u64, u64, _>(Not::not)
            .or_else(|| self.try_map::<i64, i64, _>(Not::not))
    }
}

impl Neg for StackEntry
{
    type Output = Option<Self>;
    fn neg(self) -> Self::Output
    {
        self.try_map::<i64, i64, _>(i64::wrapping_neg) // Signed Int
            .or_else(|| self.try_map::<f32, f32, _>(Neg::neg)) // Float
            .or_else(|| self.try_map::<f64, f64, _>(Neg::neg)) // Double
    }
}
