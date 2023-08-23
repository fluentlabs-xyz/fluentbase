use ethers_core::{
    k256::elliptic_curve::PrimeField,
    types::{Address, U256},
};
use halo2_proofs::{arithmetic::FieldExt, halo2curves::bn256::Fr};
use hash_circuit::hash::{Hashable, MessageHashable};
use num_bigint::BigUint;

pub trait Field: FieldExt + Hashable + MessageHashable {}

impl Field for Fr {}

pub(crate) fn hash<F: Field>(x: F, y: F) -> F {
    Hashable::hash([x, y])
}

pub(crate) trait Bit {
    fn bit(&self, i: usize) -> bool;
}

impl Bit for Fr {
    fn bit(&self, i: usize) -> bool {
        let mut bytes = self.to_bytes();
        bytes.reverse();
        bytes
            .get(31 - i / 8)
            .map_or_else(|| false, |&byte| byte & (1 << (i % 8)) != 0)
    }
}

pub(crate) fn split_word<F: Field>(x: U256) -> (F, F) {
    let mut bytes = [0; 32];
    x.to_big_endian(&mut bytes);
    let high_bytes: [u8; 16] = bytes[..16].try_into().unwrap();
    let low_bytes: [u8; 16] = bytes[16..].try_into().unwrap();

    let high = F::from_u128(u128::from_be_bytes(high_bytes));
    let low = F::from_u128(u128::from_be_bytes(low_bytes));
    (high, low)

    // TODO: what's wrong with this?
    // let [limb_0, limb_1, limb_2, limb_3] = key.0;
    // let key_high = Fr::from_u128(u128::from(limb_2) + u128::from(limb_3) << 64);
    // let key_low = Fr::from_u128(u128::from(limb_0) + u128::from(limb_1) << 64);
    // hash(key_high, key_low)
}

pub(crate) fn hi_lo<F: Field>(x: &BigUint) -> (F, F) {
    let mut u64_digits = x.to_u64_digits();
    u64_digits.resize(4, 0);
    (
        F::from_u128((u128::from(u64_digits[3]) << 64) + u128::from(u64_digits[2])),
        F::from_u128((u128::from(u64_digits[1]) << 64) + u128::from(u64_digits[0])),
    )
}

pub(crate) fn u256_hi_lo(x: &U256) -> (u128, u128) {
    let u64_digits = x.0;
    (
        (u128::from(u64_digits[3]) << 64) + u128::from(u64_digits[2]),
        (u128::from(u64_digits[1]) << 64) + u128::from(u64_digits[0]),
    )
}
pub(crate) fn fr_from_biguint(b: &BigUint) -> Fr {
    b.to_u64_digits()
        .iter()
        .rev() // to_u64_digits has least significant digit first
        .fold(Fr::zero(), |a, b| {
            a * Fr::from(1 << 32).square() + Fr::from(*b)
        })
}

pub fn rlc(be_bytes: &[u8], randomness: Fr) -> Fr {
    let x = be_bytes.iter().fold(Fr::zero(), |acc, byte| {
        randomness * acc + Fr::from(u64::from(*byte))
    });
    // dbg!(x);
    x
}

pub fn u256_from_biguint(x: &BigUint) -> U256 {
    U256::from_big_endian(&x.to_bytes_be())
}

pub fn u256_to_fr(x: U256) -> Fr {
    let mut bytes = [0u8; 32];
    x.to_little_endian(&mut bytes);
    Fr::from_repr(bytes).unwrap()
}

pub fn u256_to_big_endian(x: &U256) -> Vec<u8> {
    let mut bytes = [0; 32];
    x.to_big_endian(&mut bytes);
    bytes.to_vec()
}

pub fn storage_key_hash<F: Field>(key: U256) -> F {
    let (high, low) = split_word(key);
    hash(high, low)
}

pub fn account_key<F: Field>(address: Address) -> F {
    let high_bytes: [u8; 16] = address.0[..16].try_into().unwrap();
    let low_bytes: [u8; 4] = address.0[16..].try_into().unwrap();

    let address_high = F::from_u128(u128::from_be_bytes(high_bytes));
    let address_low = F::from_u128(u128::from(u32::from_be_bytes(low_bytes)) << 96);
    hash(address_high, address_low)
}

pub fn unroll_to_hash_input<F: FieldExt, const BYTES_IN_FIELD: usize, const INPUT_LEN: usize>(
    code: impl ExactSizeIterator<Item = u8>,
) -> Vec<[F; INPUT_LEN]> {
    let fl_cnt = code.len() / BYTES_IN_FIELD;
    let fl_cnt = if code.len() % BYTES_IN_FIELD != 0 {
        fl_cnt + 1
    } else {
        fl_cnt
    };

    let (msgs, _) = code
        .chain(std::iter::repeat(0))
        .take(fl_cnt * BYTES_IN_FIELD)
        .fold((Vec::new(), Vec::new()), |(mut msgs, mut cache), bt| {
            cache.push(bt);
            if cache.len() == BYTES_IN_FIELD {
                let mut buf: [u8; 64] = [0; 64];
                U256::from_big_endian(&cache).to_little_endian(&mut buf[0..32]);
                msgs.push(F::from_bytes_wide(&buf));
                cache.clear();
            }
            (msgs, cache)
        });

    let input_cnt = msgs.len() / INPUT_LEN;
    let input_cnt = if msgs.len() % INPUT_LEN != 0 {
        input_cnt + 1
    } else {
        input_cnt
    };
    if input_cnt == 0 {
        return Vec::new();
    }

    let (mut inputs, last) = msgs
        .into_iter()
        .chain(std::iter::repeat(F::zero()))
        .take(input_cnt * INPUT_LEN)
        .fold(
            (Vec::new(), [None; INPUT_LEN]),
            |(mut msgs, mut v_arr), f| {
                if let Some(v) = v_arr.iter_mut().find(|v| v.is_none()) {
                    v.replace(f);
                    (msgs, v_arr)
                } else {
                    msgs.push(v_arr.map(|v| v.unwrap()));
                    let mut v_arr = [None; INPUT_LEN];
                    v_arr[0].replace(f);
                    (msgs, v_arr)
                }
            },
        );

    inputs.push(last.map(|v| v.unwrap()));
    inputs
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_u256_hi_lo() {
        assert_eq!(u256_hi_lo(&U256::one()), (0, 1));
    }
}
