use crate::word_size::{
    common::MemoryMappingHelper,
    slice::{RetVal, SpecMethods},
};
use alloc::{vec, vec::Vec};

#[repr(C)]
#[derive(Copy, Clone)]
pub struct BigModExpParams {
    pub base: u64,
    pub base_len: u64,
    pub exponent: u64,
    pub exponent_len: u64,
    pub modulus: u64,
    pub modulus_len: u64,
}

impl<'a> SpecMethods<'a> for BigModExpParams {
    const ITEM_SIZE_BYTES: usize = size_of::<Self>();

    fn recover_from_bytes(
        byte_repr: &'a [u8],
        _memory_mapping_helper: MemoryMappingHelper<'a>,
    ) -> RetVal<'a, Self>
    where
        Self: Sized,
    {
        const COMPONENT_BYTES_SIZE: usize = size_of::<u64>();
        assert_eq!(
            byte_repr.len(),
            COMPONENT_BYTES_SIZE * 6,
            "incorrect byte repr len for BigModExpParams"
        );
        #[inline(always)]
        fn get_at_idx(byte_repr: &[u8], idx: usize) -> u64 {
            u64::from_le_bytes(
                byte_repr[COMPONENT_BYTES_SIZE * idx..COMPONENT_BYTES_SIZE * (idx + 1)]
                    .try_into()
                    .unwrap(),
            )
        }
        RetVal::Instance(BigModExpParams {
            base: get_at_idx(byte_repr, 0),
            base_len: get_at_idx(byte_repr, 1),
            exponent: get_at_idx(byte_repr, 2),
            exponent_len: get_at_idx(byte_repr, 3),
            modulus: get_at_idx(byte_repr, 4),
            modulus_len: get_at_idx(byte_repr, 5),
        })
    }
}

/// Big integer modular exponentiation
pub fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &[u8]) -> Vec<u8> {
    {
        use num_bigint::BigUint;
        use num_traits::{One, Zero};

        let modulus_len = modulus.len();
        let base = BigUint::from_bytes_be(base);
        let exponent = BigUint::from_bytes_be(exponent);
        let modulus = BigUint::from_bytes_be(modulus);

        if modulus.is_zero() || modulus.is_one() {
            return vec![0_u8; modulus_len];
        }

        let ret_int = base.modpow(&exponent, &modulus);
        let ret_int = ret_int.to_bytes_be();
        let mut return_value = vec![0_u8; modulus_len.saturating_sub(ret_int.len())];
        return_value.extend(ret_int);
        return_value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_mod_exp_test() {
        #[derive(serde_derive::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct TestCase {
            base: String,
            exponent: String,
            modulus: String,
            expected: String,
        }

        let test_data = r#"[
        {
            "Base":     "1111111111111111111111111111111111111111111111111111111111111111",
            "Exponent": "1111111111111111111111111111111111111111111111111111111111111111",
            "Modulus":  "111111111111111111111111111111111111111111111111111111111111110A",
            "Expected": "0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD"
        },
        {
            "Base":     "2222222222222222222222222222222222222222222222222222222222222222",
            "Exponent": "2222222222222222222222222222222222222222222222222222222222222222",
            "Modulus":  "1111111111111111111111111111111111111111111111111111111111111111",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        {
            "Base":     "3333333333333333333333333333333333333333333333333333333333333333",
            "Exponent": "3333333333333333333333333333333333333333333333333333333333333333",
            "Modulus":  "2222222222222222222222222222222222222222222222222222222222222222",
            "Expected": "1111111111111111111111111111111111111111111111111111111111111111"
        },
        {
            "Base":     "9874231472317432847923174392874918237439287492374932871937289719",
            "Exponent": "0948403985401232889438579475812347232099080051356165126166266222",
            "Modulus":  "25532321a214321423124212222224222b242222222222222222222222222444",
            "Expected": "220ECE1C42624E98AEE7EB86578B2FE5C4855DFFACCB43CCBB708A3AB37F184D"
        },
        {
            "Base":     "3494396663463663636363662632666565656456646566786786676786768766",
            "Exponent": "2324324333246536456354655645656616169896565698987033121934984955",
            "Modulus":  "0218305479243590485092843590249879879842313131156656565565656566",
            "Expected": "012F2865E8B9E79B645FCE3A9E04156483AE1F9833F6BFCF86FCA38FC2D5BEF0"
        },
        {
            "Base":     "0000000000000000000000000000000000000000000000000000000000000005",
            "Exponent": "0000000000000000000000000000000000000000000000000000000000000002",
            "Modulus":  "0000000000000000000000000000000000000000000000000000000000000007",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000004"
        },
        {
            "Base":     "0000000000000000000000000000000000000000000000000000000000000019",
            "Exponent": "0000000000000000000000000000000000000000000000000000000000000019",
            "Modulus":  "0000000000000000000000000000000000000000000000000000000000000064",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000019"
        },
        {
            "Base":     "0000000000000000000000000000000000000000000000000000000000000019",
            "Exponent": "0000000000000000000000000000000000000000000000000000000000000019",
            "Modulus":  "0000000000000000000000000000000000000000000000000000000000000000",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        {
            "Base":     "0000000000000000000000000000000000000000000000000000000000000019",
            "Exponent": "0000000000000000000000000000000000000000000000000000000000000019",
            "Modulus":  "0000000000000000000000000000000000000000000000000000000000000001",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000000"
        }
        ]"#;

        let test_cases: Vec<TestCase> = serde_json::from_str(test_data).unwrap();
        test_cases.iter().for_each(|test| {
            let base = array_bytes::hex2bytes_unchecked(&test.base);
            let exponent = array_bytes::hex2bytes_unchecked(&test.exponent);
            let modulus = array_bytes::hex2bytes_unchecked(&test.modulus);
            let expected = array_bytes::hex2bytes_unchecked(&test.expected);
            let result = big_mod_exp(base.as_slice(), exponent.as_slice(), modulus.as_slice());
            assert_eq!(result, expected);
        });
    }
}
