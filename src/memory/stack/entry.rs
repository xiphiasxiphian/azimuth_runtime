use std::{ops::{
    Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not, Rem, Shl,
    Shr, Sub,
}, ptr::NonNull};

use crate::memory::stack::convert::StackableConvert;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum StackEntry
{
    Unsigned(u64),
    Signed(i64),
    Character(char),
    Float(f32),
    Double(f64),
    Reference(Option<NonNull<u8>>)
}

impl StackEntry
{
    pub fn try_binary_operation<T, F>(self, other: Self, op: F) -> Option<Self>
    where
        Self: TryInto<T>,
        T: Into<StackEntry>,
        F: Fn(T, T) -> T
    {
        let first = self.try_into().ok()?;
        let second = other.try_into().ok()?;

        Some(op(first, second).into())
    }

    pub fn try_map<T1, T2, F>(self, op: F) -> Option<Self>
    where
        Self: TryInto<T1>,
        T2: Into<StackEntry>,
        F: Fn(T1) -> T2
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
}

macro_rules! impl_elementwise_trait {
    ($($id:ident($t:expr => $($r:tt),+)),*) => {
        impl StackEntry
        {
            $(
                fn $id(&self, other: &Self) -> Option<Self>
                {
                    match (*self, *other)
                    {
                        $(
                            (Self::$r(x), Self::$r(y)) => Some(Self::$r($t(x, y))),
                        )+
                        _ => None
                    }
                }
            )*
        }
    };
    (~ $($id:ident($t:expr => $($r:tt),+)),*) => {
        impl StackEntry
        {
            $(
                fn $id(self) -> Option<Self>
                {
                    match self
                    {
                        $(
                            Self::$r(x) => Some(Self::$r($t(x))),
                        )+
                        _ => None
                    }
                }
            )*
        }
    };
    (~~ $($id:ident($t:expr => $($r:tt),+)),*) => {
        impl StackEntry
        {
            $(
                fn $id(self, y: usize) -> Option<Self>
                {
                    match self
                    {
                        $(
                            Self::$r(x) => Some(Self::$r($t(x, y))),
                        )+
                        _ => None
                    }
                }
            )*
        }
    };
}

impl_elementwise_trait!(
    try_add(Add::add => Unsigned, Signed, Float, Double),
    try_sub(Sub::sub => Unsigned, Signed, Float, Double),
    try_mul(Mul::mul => Unsigned, Signed, Float, Double),
    try_div(Div::div => Unsigned, Signed, Float, Double),
    try_rem(Rem::rem => Unsigned, Signed, Float, Double),
    try_bitor(BitOr::bitor => Unsigned, Signed),
    try_bitand(BitAnd::bitand => Unsigned, Signed),
    try_bitxor(BitXor::bitxor => Unsigned, Signed)
);

impl_elementwise_trait!(~
    try_not(Not::not => Unsigned, Signed),
    try_neg(Neg::neg => Signed, Float, Double)
);

impl_elementwise_trait!(~~
    try_shr(Shr::shr => Unsigned, Signed),
    try_shl(Shl::shl => Unsigned, Signed)
);
