use super::BinaryQuery;
use crate::{
    constraint_builder::{AdviceColumn, AdviceColumnPhase2, FixedColumn},
    util::Field,
};
use ethers_core::k256::elliptic_curve::PrimeField;
use halo2_proofs::{
    halo2curves::bn256::Fr,
    plonk::{Advice, Challenge, Column, Expression, Fixed, Instance, VirtualCells},
    poly::Rotation,
};
use num_bigint::BigUint;

#[derive(Clone, Copy)]
pub enum ColumnType {
    Advice,
    Fixed,
    Challenge,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Query<F: Field> {
    Constant(F),
    Advice(Column<Advice>, i32),
    Instance(Column<Instance>, i32),
    Fixed(Column<Fixed>, i32),
    Challenge(Challenge),
    Neg(Box<Self>),
    Add(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
}

impl<F: Field> Default for Query<F> {
    fn default() -> Self {
        Self::Constant(F::zero())
    }
}

impl<F: Field> Query<F> {
    pub fn zero() -> Self {
        Self::from(0)
    }

    pub fn one() -> Self {
        Self::from(1)
    }

    fn two_to_the_64th() -> Self {
        Self::from(1 << 32).square()
    }

    pub fn run(&self, meta: &mut VirtualCells<'_, F>) -> Expression<F> {
        match self {
            Query::Constant(f) => Expression::Constant(*f),
            Query::Advice(c, r) => meta.query_advice(*c, Rotation(*r)),
            Query::Instance(c, r) => meta.query_instance(*c, Rotation(*r)),
            Query::Fixed(c, r) => meta.query_fixed(*c, Rotation(*r)),
            Query::Challenge(c) => meta.query_challenge(*c),
            Query::Neg(q) => Expression::Constant(F::zero()) - q.run(meta),
            Query::Add(q, u) => q.run(meta) + u.run(meta),
            Query::Mul(q, u) => q.run(meta) * u.run(meta),
        }
    }

    pub fn square(self) -> Self {
        self.clone() * self
    }
}

impl<F: Field> From<u64> for Query<F> {
    fn from(x: u64) -> Self {
        Self::Constant(F::from(x))
    }
}

pub fn bn_to_field<F: Field>(bn: &BigUint) -> F {
    let mut bytes = bn.to_bytes_le();
    bytes.resize(64, 0);
    F::from_bytes_wide(&bytes.try_into().unwrap())
}

impl<F: Field> Query<F> {
    pub fn from_bn(x: &BigUint) -> Self {
        Self::Constant(bn_to_field(x))
    }
}

impl<F: Field> From<Fr> for Query<F> {
    fn from(x: Fr) -> Self {
        let little_endian_bytes = x.to_repr();
        let little_endian_limbs = little_endian_bytes
            .as_slice()
            .chunks_exact(8)
            .map(|s| u64::from_le_bytes(s.try_into().unwrap()));
        little_endian_limbs.rfold(Query::zero(), |result, limb| {
            result * Query::two_to_the_64th() + limb
        })
    }
}

impl<F: Field> From<BinaryQuery<F>> for Query<F> {
    fn from(b: BinaryQuery<F>) -> Self {
        b.0
    }
}

impl<F: Field, T: Into<Query<F>>> std::ops::Add<T> for Query<F> {
    type Output = Self;
    fn add(self, other: T) -> Self::Output {
        Self::Add(Box::new(self), Box::new(other.into()))
    }
}

impl<F: Field, T: Into<Query<F>>> std::ops::Sub<T> for Query<F> {
    type Output = Self;
    fn sub(self, other: T) -> Self::Output {
        Self::Add(Box::new(self), Box::new(Self::Neg(Box::new(other.into()))))
    }
}

impl<F: Field, T: Into<Query<F>>> std::ops::Mul<T> for Query<F> {
    type Output = Self;
    fn mul(self, other: T) -> Self::Output {
        Self::Mul(Box::new(self), Box::new(other.into()))
    }
}

pub trait ToExpr {
    fn expr<F: Field>(&self) -> Query<F>;

    fn query<F: Field>(&self) -> Query<F> {
        self.expr()
    }
}

#[macro_export]
macro_rules! impl_expr {
    (RwTableContextTag) => {
        impl $crate::constraint_builder::ToExpr for RwTableContextTag {
            fn expr<F: $crate::util::Field>(&self) -> $crate::constraint_builder::Query<F> {
                $crate::constraint_builder::Query::from(Into::<u32>::into(*self) as u64)
            }
        }
    };
    ($ty:ty) => {
        impl $crate::constraint_builder::ToExpr for $ty {
            fn expr<F: $crate::util::Field>(&self) -> $crate::constraint_builder::Query<F> {
                $crate::constraint_builder::Query::from(*self as u64)
            }
        }
    };
}

impl_expr!(u64);
impl_expr!(i64);
impl_expr!(u32);
impl_expr!(i32);
impl_expr!(u16);
impl_expr!(i16);
impl_expr!(u8);
impl_expr!(i8);
impl_expr!(usize);
impl_expr!(isize);

impl ToExpr for AdviceColumn {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}
impl ToExpr for AdviceColumnPhase2 {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}
impl ToExpr for FixedColumn {
    fn expr<F: Field>(&self) -> Query<F> {
        self.current()
    }
}

// /// Returns the sum of the passed in cells
// pub mod sum {
//     use crate::{
//         constraint_builder::{query::ToExpr as Expr, Query as Expression},
//         util::Field,
//     };
//
//     /// Returns an expression for the sum of the list of expressions.
//     pub fn expr<F: Field, E: Expr, I: IntoIterator<Item = E>>(inputs: I) -> Expression<F> {
//         inputs
//             .into_iter()
//             .fold(0.expr(), |acc, input| acc + input.expr())
//     }
//
//     /// Returns the sum of the given list of values within the field.
//     pub fn value<F: Field>(values: &[u8]) -> F {
//         values
//             .iter()
//             .fold(F::zero(), |acc, value| acc + F::from(*value as u64))
//     }
// }
//
// /// Returns `1` when `expr[0] && expr[1] && ... == 1`, and returns `0`
// /// otherwise. Inputs need to be boolean
// pub mod and {
//     use crate::{
//         constraint_builder::{query::ToExpr as Expr, Query as Expression},
//         util::Field,
//     };
//
//     /// Returns an expression that evaluates to 1 only if all the expressions in
//     /// the given list are 1, else returns 0.
//     pub fn expr<F: Field, E: Expr, I: IntoIterator<Item = E>>(inputs: I) -> Expression<F> {
//         inputs
//             .into_iter()
//             .fold(1.expr(), |acc, input| acc * input.expr())
//     }
//
//     /// Returns the product of all given values.
//     pub fn value<F: Field>(inputs: Vec<F>) -> F {
//         inputs.iter().fold(F::one(), |acc, input| acc * input)
//     }
// }
//
// /// Returns `1` when `expr[0] || expr[1] || ... == 1`, and returns `0`
// /// otherwise. Inputs need to be boolean
// pub mod or {
//     use crate::{
//         constraint_builder::{
//             query::{and, not, ToExpr as Expr},
//             Query as Expression,
//         },
//         util::Field,
//     };
//
//     /// Returns an expression that evaluates to 1 if any expression in the given
//     /// list is 1. Returns 0 if all the expressions were 0.
//     pub fn expr<F: Field, E: Expr, I: IntoIterator<Item = E>>(inputs: I) -> Expression<F> {
//         not::expr(and::expr(inputs.into_iter().map(not::expr)))
//     }
//
//     /// Returns the value after passing all given values through the OR gate.
//     pub fn value<F: Field>(inputs: Vec<F>) -> F {
//         not::value(and::value(inputs.into_iter().map(not::value).collect()))
//     }
// }
//
// /// Returns `1` when `b == 0`, and returns `0` otherwise.
// /// `b` needs to be boolean
// pub mod not {
//     use crate::{
//         constraint_builder::{query::ToExpr as Expr, Query as Expression},
//         util::Field,
//     };
//
//     /// Returns an expression that represents the NOT of the given expression.
//     pub fn expr<F: Field, E: Expr>(b: E) -> Expression<F> {
//         1.expr() - b.expr()
//     }
//
//     /// Returns a value that represents the NOT of the given value.
//     pub fn value<F: Field>(b: F) -> F {
//         F::one() - b
//     }
// }
//
// /// Returns `a ^ b`.
// /// `a` and `b` needs to be boolean
// pub mod xor {
//     use crate::{
//         constraint_builder::{query::ToExpr as Expr, Query as Expression},
//         util::Field,
//     };
//
//     /// Returns an expression that represents the XOR of the given expression.
//     pub fn expr<F: Field, E: Expr>(a: E, b: E) -> Expression<F> {
//         a.expr() + b.expr() - 2.expr() * a.expr() * b.expr()
//     }
//
//     /// Returns a value that represents the XOR of the given value.
//     pub fn value<F: Field>(a: F, b: F) -> F {
//         a + b - F::from(2u64) * a * b
//     }
// }
//
// /// Returns `when_true` when `selector == 1`, and returns `when_false` when
// /// `selector == 0`. `selector` needs to be boolean.
// pub mod select {
//     use crate::{
//         constraint_builder::{query::ToExpr, Query},
//         util::Field,
//     };
//
//     /// Returns the `when_true` expression when the selector is true, else
//     /// returns the `when_false` expression.
//     pub fn expr<F: Field>(
//         selector: Query<F>,
//         when_true: Query<F>,
//         when_false: Query<F>,
//     ) -> Query<F> { selector.clone() * when_true + (1.expr() - selector) * when_false
//     }
//
//     /// Returns the `when_true` value when the selector is true, else returns
//     /// the `when_false` value.
//     pub fn value<F: Field>(selector: F, when_true: F, when_false: F) -> F {
//         selector * when_true + (F::one() - selector) * when_false
//     }
//
//     /// Returns the `when_true` word when selector is true, else returns the
//     /// `when_false` word.
//     pub fn value_word<F: Field>(
//         selector: F,
//         when_true: [u8; 32],
//         when_false: [u8; 32],
//     ) -> [u8; 32] { if selector == F::one() { when_true } else { when_false }
//     }
// }
