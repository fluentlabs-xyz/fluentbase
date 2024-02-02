use super::{p128pow5t3::P128Pow5T3Constants, Mds};
pub(crate) use halo2curves::bn256::Fr as Fp;

pub(crate) mod fp;

impl P128Pow5T3Constants for Fp {
    fn partial_rounds() -> usize {
        57
    }

    fn round_constants() -> Vec<[Fp; 3]> {
        fp::ROUND_CONSTANTS.to_vec()
    }
    fn mds() -> Mds<Fp, 3> {
        *fp::MDS
    }
    fn mds_inv() -> Mds<Fp, 3> {
        *fp::MDS_INV
    }
}
