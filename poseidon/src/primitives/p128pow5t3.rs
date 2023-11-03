use super::{Mds, Spec};
use halo2curves::FieldExt;
use std::marker::PhantomData;

/// The trait required for fields can handle a pow5 sbox, 3 field, 2 rate permutation
pub trait P128Pow5T3Constants: FieldExt {
    fn partial_rounds() -> usize {
        56
    }
    fn round_constants() -> Vec<[Self; 3]>;
    fn mds() -> Mds<Self, 3>;
    fn mds_inv() -> Mds<Self, 3>;
}

/// Poseidon-128 using the $x^5$ S-box, with a width of 3 field elements, and the
/// standard number of rounds for 128-bit security "with margin".
///
/// The standard specification for this set of parameters (on either of the Pasta
/// fields) uses $R_F = 8, R_P = 56$. This is conveniently an even number of
/// partial rounds, making it easier to construct a Halo 2 circuit.
#[derive(Debug)]
pub struct P128Pow5T3<C> {
    _marker: PhantomData<C>,
}

impl<Fp: P128Pow5T3Constants> Spec<Fp, 3, 2> for P128Pow5T3<Fp> {
    fn full_rounds() -> usize {
        8
    }

    fn partial_rounds() -> usize {
        Fp::partial_rounds()
    }

    fn sbox(val: Fp) -> Fp {
        val.pow_vartime([5])
    }

    fn secure_mds() -> usize {
        unimplemented!()
    }

    fn constants() -> (Vec<[Fp; 3]>, Mds<Fp, 3>, Mds<Fp, 3>) {
        (Fp::round_constants(), Fp::mds(), Fp::mds_inv())
    }
}
