use fluentbase_crypto::utils::{
    bytes_to_words_le, words_to_bytes_le, AffinePoint, WeierstrassAffinePoint,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestPoint {
    Infinity,
    Affine([u32; 4]),
}

impl AffinePoint<4> for TestPoint {
    const GENERATOR: [u32; 4] = [1, 2, 3, 4];
    const GENERATOR_T: Self = Self::Affine([1, 2, 3, 4]);

    fn new(limbs: [u32; 4]) -> Self {
        Self::Affine(limbs)
    }

    fn identity() -> Self {
        Self::Infinity
    }

    fn limbs_ref(&self) -> &[u32; 4] {
        match self {
            Self::Infinity => panic!("infinity has no affine limbs"),
            Self::Affine(limbs) => limbs,
        }
    }

    fn limbs_mut(&mut self) -> &mut [u32; 4] {
        match self {
            Self::Infinity => panic!("infinity has no affine limbs"),
            Self::Affine(limbs) => limbs,
        }
    }

    fn is_identity(&self) -> bool {
        matches!(self, Self::Infinity)
    }

    fn add_assign(&mut self, other: &Self) {
        if self.is_identity() {
            *self = other.clone();
            return;
        }
        if other.is_identity() {
            return;
        }

        for (limb, other_limb) in self.limbs_mut().iter_mut().zip(other.limbs_ref()) {
            *limb = limb.wrapping_add(*other_limb);
        }
    }

    fn double(&mut self) {
        if let Self::Affine(limbs) = self {
            for limb in limbs {
                *limb = limb.wrapping_mul(2);
            }
        }
    }
}

impl WeierstrassAffinePoint<4> for TestPoint {
    fn infinity() -> Self {
        Self::Infinity
    }

    fn is_infinity(&self) -> bool {
        self.is_identity()
    }
}

#[test]
fn converts_limbs_and_coordinates() {
    let words = [0x1122_3344, 0xaabb_ccdd, 0x0102_0304, 0xdead_beef];
    let bytes = words_to_bytes_le(&words);
    assert_eq!(bytes_to_words_le(&bytes), words);
    assert!(bytes_to_words_le(&bytes[..bytes.len() - 1]).len() < words.len());

    let from_coordinates = <TestPoint as AffinePoint<4>>::from(&bytes[..8], &bytes[8..]);
    assert_eq!(from_coordinates, TestPoint::Affine(words));
    assert_eq!(TestPoint::from_le_bytes(&bytes), TestPoint::Affine(words));
    assert_eq!(from_coordinates.to_le_bytes(), bytes);
}

#[test]
fn multiplies_points_and_computes_msm() {
    let mut point = TestPoint::Affine([1, 2, 3, 4]);
    point.mul_assign(&[3, 0]);
    assert_eq!(point, TestPoint::Affine([3, 6, 9, 12]));

    let result = TestPoint::multi_scalar_multiplication(
        &[true, false, true],
        TestPoint::Affine([1, 2, 3, 4]),
        &[false, true, true],
        TestPoint::Affine([10, 20, 30, 40]),
    );
    assert_eq!(result, TestPoint::Affine([65, 130, 195, 260]));
}

#[test]
fn handles_all_complete_weierstrass_addition_cases() {
    let other = TestPoint::Affine([1, 2, 3, 4]);

    let mut infinity = TestPoint::Infinity;
    infinity.weierstrass_add_assign(&other);
    assert_eq!(infinity, other);

    let mut unchanged = other.clone();
    unchanged.weierstrass_add_assign(&TestPoint::Infinity);
    assert_eq!(unchanged, other);

    let mut doubled = other.clone();
    doubled.weierstrass_add_assign(&other);
    assert_eq!(doubled, TestPoint::Affine([2, 4, 6, 8]));

    let mut inverse = TestPoint::Affine([1, 2, 3, 4]);
    inverse.weierstrass_add_assign(&TestPoint::Affine([1, 2, 5, 6]));
    assert_eq!(inverse, TestPoint::Infinity);

    let mut sum = TestPoint::Affine([1, 2, 3, 4]);
    sum.weierstrass_add_assign(&TestPoint::Affine([5, 6, 7, 8]));
    assert_eq!(sum, TestPoint::Affine([6, 8, 10, 12]));
}
