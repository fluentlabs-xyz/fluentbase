use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query, ToExpr},
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};

/// Returns `1` when `lhs < rhs`, and returns `0` otherwise.
/// lhs and rhs `< 256**N_BYTES`
/// `N_BYTES` is required to be `<= MAX_N_BYTES_INTEGER` to prevent overflow:
/// values are stored in a single field element and two of these are added
/// together.
/// The equation that is enforced is `lhs - rhs == diff - (lt * range)`.
/// Because all values are `<= 256**N_BYTES` and `lt` is boolean, `lt` can only
/// be `1` when `lhs < rhs`.
#[derive(Clone, Debug)]
pub struct LtGadget<F: Field, const N_BYTES: usize> {
    lt: AdviceColumn, // `1` when `lhs < rhs`, `0` otherwise.
    diff: [AdviceColumn; N_BYTES], /* The byte values of `diff`.
                       * `diff` equals `lhs - rhs` if `lhs >= rhs`,
                       * `lhs - rhs + range` otherwise. */
    range: F, // The range of the inputs, `256**N_BYTES`
}

pub(crate) fn from_bytes_expr<F: Field, E: ToExpr<F>>(bytes: &[E]) -> Query<F> {
    debug_assert!(
        bytes.len() <= 31,
        "Too many bytes to compose an integer in field"
    );
    let mut value = 0.expr();
    let mut multiplier = F::one();
    for byte in bytes.iter() {
        value = value + byte.expr() * Query::Constant(multiplier);
        multiplier *= F::from(256);
    }
    value
}

impl<F: Field, const N_BYTES: usize> LtGadget<F, N_BYTES> {
    pub(crate) fn configure(
        cs: &mut ConstraintSystem<F>,
        cb: &mut ConstraintBuilder<F>,
        lhs: Query<F>,
        rhs: Query<F>,
    ) -> Self {
        assert!(N_BYTES <= 31);
        let lt = AdviceColumn(cs.advice_column());
        let diff: [AdviceColumn; N_BYTES] = cb.advice_columns(cs);
        let range = F::from(2).pow(&[8 * N_BYTES as u64, 0, 0, 0]);

        // The equation we require to hold: `lhs - rhs == diff - (lt * range)`.
        cb.assert_equal(
            "lhs - rhs == diff - (lt ⋅ range)",
            lhs - rhs,
            from_bytes_expr(&diff) - (lt.expr() * Query::Constant(range)),
        );

        Self { lt, diff, range }
    }

    pub(crate) fn expr(&self) -> Query<F> {
        self.lt.expr()
    }

    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        lhs: F,
        rhs: F,
    ) -> (F, Vec<u8>) {
        // Set `lt`
        let lt = lhs < rhs;
        self.lt
            .assign(region, offset, if lt { F::one() } else { F::zero() });

        // Set the bytes of diff
        let diff = (lhs - rhs) + (if lt { self.range } else { F::zero() });
        let diff_bytes = diff.to_repr();
        for (idx, diff) in self.diff.iter().enumerate() {
            diff.assign(region, offset, diff_bytes.as_ref()[idx] as u64);
        }

        (
            if lt { F::one() } else { F::zero() },
            diff_bytes.as_ref().to_vec(),
        )
    }

    pub(crate) fn diff_bytes(&self) -> Vec<AdviceColumn> {
        self.diff.to_vec()
    }
}
