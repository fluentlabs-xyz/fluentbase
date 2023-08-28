//! The binary number chip implements functionality to represent any given value
//! in binary bits, which can be compared against a value or expression for
//! equality.

use crate::{
    constraint_builder::{and, not, AdviceColumn, FixedColumn, Query, ToExpr},
    util::Field,
};
use halo2_proofs::{
    circuit::{Region, Value},
    plonk::{Any, Column, ConstraintSystem, Error, Fixed},
    poly::Rotation,
};
use std::{collections::BTreeSet, marker::PhantomData};
use strum::IntoEnumIterator;

/// Helper trait that implements functionality to represent a generic type as
/// array of N-bits.
pub trait AsBits<const N: usize> {
    /// Return the bits of self, starting from the most significant.
    fn as_bits(&self) -> [bool; N];
}

impl<T, const N: usize> AsBits<N> for T
where
    T: Copy + Into<usize>,
{
    fn as_bits(&self) -> [bool; N] {
        let mut bits = [false; N];
        let mut x: usize = (*self).into();
        for i in 0..N {
            bits[N - 1 - i] = x % 2 == 1;
            x /= 2;
        }
        bits
    }
}

/// Config for the binary number chip.
#[derive(Clone, Copy, Debug)]
pub struct BinaryNumberConfig<T, const N: usize> {
    /// Must be constrained to be binary for correctness.
    pub bits: [AdviceColumn; N],
    _marker: PhantomData<T>,
}

impl<T, const N: usize> BinaryNumberConfig<T, N>
where
    T: AsBits<N>,
{
    /// Returns the expression value of the bits at the given rotation.
    pub fn value<F: Field>(&self, rotation: Rotation) -> impl FnOnce() -> Query<F> {
        let bits = self.bits;
        move || {
            let bits = bits.map(|bit| bit.rotation(rotation.0));
            bits.iter()
                .fold(0.expr(), |result, bit| bit.clone() + result * 2.expr())
        }
    }

    /// Returns a function that can evaluate to a binary expression, that
    /// evaluates to 1 if value is equal to value as bits. The returned
    /// expression is of degree N.
    pub fn value_equals<F: Field, S: AsBits<N>>(
        &self,
        value: S,
        rotation: Rotation,
    ) -> impl FnOnce() -> Query<F> {
        let bits = self.bits;
        move || Self::value_equals_expr(value, bits.map(|bit| bit.rotation(rotation.0)))
    }

    /// Returns a binary expression that evaluates to 1 if expressions are equal
    /// to value as bits. The returned expression is of degree N.
    pub fn value_equals_expr<F: Field, S: AsBits<N>>(
        value: S,
        expressions: [Query<F>; N], // must be binary.
    ) -> Query<F> {
        and::expr(
            value
                .as_bits()
                .iter()
                .zip(&expressions)
                .map(|(&bit, expression)| {
                    if bit {
                        expression.clone()
                    } else {
                        not::expr(expression.clone())
                    }
                }),
        )
    }

    /// Annotates columns of this gadget embedded within a circuit region.
    pub fn annotate_columns_in_region<F: Field>(&self, region: &mut Region<F>, prefix: &str) {
        let mut annotations = Vec::new();
        for (i, _) in self.bits.iter().enumerate() {
            annotations.push(format!("GADGETS_binary_number_{}", i));
        }
        self.bits
            .iter()
            .zip(annotations.iter())
            .for_each(|(col, ann)| region.name_column(|| format!("{}_{}", prefix, ann), *col));
    }
}

/// This chip helps working with binary encoding of integers of length N bits
/// by:
///  - enforcing that the binary representation is in the valid range defined by T.
///  - creating expressions (via the Config) that evaluate to 1 when the bits match a specific value
///    and 0 otherwise.
#[derive(Clone, Debug)]
pub struct BinaryNumberChip<F, T, const N: usize> {
    ///
    pub config: BinaryNumberConfig<T, N>,
    _marker: PhantomData<F>,
}

impl<F: Field, T: IntoEnumIterator, const N: usize> BinaryNumberChip<F, T, N>
where
    T: AsBits<N>,
{
    /// Construct the binary number chip given a config.
    pub fn construct(config: BinaryNumberConfig<T, N>) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }

    /// Configure constraints for the binary number chip.
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        selector: FixedColumn,
        value: Option<Column<Any>>,
    ) -> BinaryNumberConfig<T, N> {
        let bits = [0; N].map(|_| AdviceColumn(cs.advice_column()));
        bits.map(|bit| {
            cs.create_gate("bit column is 0 or 1", |meta| {
                vec![selector.current() * bit.current() * (1.expr() - bit.current())]
            })
        });

        let config = BinaryNumberConfig {
            bits,
            _marker: PhantomData,
        };

        if let Some(value) = value {
            cs.create_gate("binary number value", |meta| {
                vec![
                    selector.current()
                        * (config.value(Rotation::cur())(meta)
                            - meta.query_any(value, Rotation::cur())),
                ]
            });
        }

        // Disallow bit patterns (if any) that don't correspond to a variant of T.
        let valid_values: BTreeSet<usize> = T::iter().map(|t| from_bits(&t.as_bits())).collect();
        let mut invalid_values = (0..1 << N).filter(|i| !valid_values.contains(i)).peekable();
        if invalid_values.peek().is_some() {
            cs.create_gate("binary number value in range", |meta| {
                let selector = selector.current();
                invalid_values
                    .map(|i| {
                        let value_equals_i = config.value_equals(i, Rotation::cur());
                        selector.clone() * value_equals_i(meta)
                    })
                    .collect::<Vec<_>>()
            });
        }

        config
    }

    /// Assign a value to the binary number chip. A generic type that implements
    /// the AsBits trait can be provided for assignment.
    pub fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: &T,
    ) -> Result<(), Error> {
        for (&bit, &column) in value.as_bits().iter().zip(&self.config.bits) {
            column.assign(region, offset, F::from(bit as u64));
        }
        Ok(())
    }
}

/// Helper function to get a decimal representation given the bits.
pub fn from_bits(bits: &[bool]) -> usize {
    bits.iter()
        .fold(0, |result, &bit| bit as usize + 2 * result)
}
