use super::BinaryQuery;
use ethers_core::k256::elliptic_curve::PrimeField;
use halo2_proofs::{
    arithmetic::{Field, FieldExt},
    halo2curves::bn256::Fr,
    plonk::{Advice, Challenge, Column, Expression, Fixed, VirtualCells},
    poly::Rotation,
};

#[derive(Clone, Copy)]
pub enum ColumnType {
    Advice,
    Fixed,
    Challenge,
}

#[derive(Clone)]
pub enum Query<F: Clone> {
    Constant(F),
    Advice(Column<Advice>, i32),
    Fixed(Column<Fixed>, i32),
    Challenge(Challenge),
    Neg(Box<Self>),
    Add(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
}

impl<F: FieldExt> Query<F> {
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

impl<F: FieldExt> From<u64> for Query<F> {
    fn from(x: u64) -> Self {
        Self::Constant(F::from(x))
    }
}

impl<F: FieldExt> From<Fr> for Query<F> {
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

impl<F: FieldExt> From<BinaryQuery<F>> for Query<F> {
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
