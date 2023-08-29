use ethers_core::types::U256;
use halo2_proofs::{arithmetic::FieldExt, halo2curves::bn256::Fr};
use poseidon_circuit::hash::{Hashable, MessageHashable};

pub trait Field: FieldExt + Hashable + MessageHashable {}

impl Field for Fr {}

pub(crate) fn poseidon_domain<F: Field>() -> F {
    F::zero()
}

pub(crate) fn poseidon_hash<F: Field>(x: F, y: F) -> F {
    Hashable::hash_with_domain([x, y], poseidon_domain::<F>())
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
