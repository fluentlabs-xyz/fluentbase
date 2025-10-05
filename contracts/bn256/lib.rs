#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice, crypto::CryptoRuntime, entrypoint, Bytes, ContextReader, CryptoAPI, ExitCode,
    SharedAPI, BN254_G1_RAW_AFFINE_SIZE, BN254_G2_RAW_AFFINE_SIZE, PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL, PRECOMPILE_BN256_PAIR,
};

/// BN254 Specific Constants
pub const SCALAR_SIZE: usize = 32;
pub const BN254_MUL_INPUT_SIZE: usize = BN254_G1_RAW_AFFINE_SIZE + SCALAR_SIZE;
pub const BN254_ADD_INPUT_SIZE: usize = 2 * BN254_G1_RAW_AFFINE_SIZE;
pub const BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN: usize =
    BN254_G1_RAW_AFFINE_SIZE + BN254_G2_RAW_AFFINE_SIZE;

/// =============== Constants ==================
///  BN256 precompile constants (EIP-196, EIP-197)
/// ============================================

const ISTANBUL_ADD_GAS_COST: u64 = 150;
const ISTANBUL_MUL_GAS_COST: u64 = 6000;
const ISTANBUL_PAIR_BASE: u64 = 45000;
const ISTANBUL_PAIR_PER_POINT: u64 = 34000;

/// Right-pad input to a specified length with zeros
#[inline(always)]
fn right_pad<const N: usize>(input: &[u8]) -> [u8; N] {
    let mut result = [0u8; N];
    let to_copy = core::cmp::min(N, input.len());
    result[..to_copy].copy_from_slice(&input[..to_copy]);
    result
}

/// Helper function for common validation and gas checking pattern
#[inline(always)]
fn validate_and_consume_gas<SDK: SharedAPI>(sdk: &SDK, gas_cost: u64, gas_limit: u64) {
    check_gas_and_sync(sdk, gas_cost, gas_limit);
}

#[inline(always)]
fn check_gas_and_sync<SDK: SharedAPI>(sdk: &SDK, gas_used: u64, gas_limit: u64) {
    if gas_used > gas_limit {
        sdk.native_exit(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_used, 0);
}

#[inline(always)]
fn read_g1_point(input: &[u8]) -> Result<[u8; BN254_G1_RAW_AFFINE_SIZE], ExitCode> {
    if input.len() != BN254_G1_RAW_AFFINE_SIZE {
        return Err(ExitCode::InputOutputOutOfBounds);
    }
    let mut g1 = [0u8; BN254_G1_RAW_AFFINE_SIZE];
    g1[..BN254_G1_RAW_AFFINE_SIZE].copy_from_slice(input);
    Ok(g1)
}

#[inline(always)]
fn read_g2_point(input: &[u8]) -> Result<[u8; BN254_G2_RAW_AFFINE_SIZE], ExitCode> {
    if input.len() != BN254_G2_RAW_AFFINE_SIZE {
        return Err(ExitCode::InputOutputOutOfBounds);
    }
    let mut g2 = [0u8; BN254_G2_RAW_AFFINE_SIZE];
    g2[..BN254_G2_RAW_AFFINE_SIZE].copy_from_slice(input);
    Ok(g2)
}

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    // read full input data
    let bytecode_address = sdk.context().contract_bytecode_address();
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    match bytecode_address {
        PRECOMPILE_BN256_ADD => {
            validate_and_consume_gas(&sdk, ISTANBUL_ADD_GAS_COST, gas_limit);
            let padded_input = right_pad::<BN254_ADD_INPUT_SIZE>(&input);

            let p: [u8; BN254_G1_RAW_AFFINE_SIZE] =
                padded_input[..BN254_G1_RAW_AFFINE_SIZE].try_into().unwrap();
            let q: [u8; BN254_G1_RAW_AFFINE_SIZE] =
                padded_input[BN254_G1_RAW_AFFINE_SIZE..].try_into().unwrap();

            let result = CryptoRuntime::bn254_add(p, q);
            sdk.write(&result);
        }
        PRECOMPILE_BN256_MUL => {
            validate_and_consume_gas(&sdk, ISTANBUL_MUL_GAS_COST, gas_limit);
            let padded_input = right_pad::<BN254_MUL_INPUT_SIZE>(&input);

            // Pass inputs as big-endian; runtime handles conversions internally
            let p: [u8; BN254_G1_RAW_AFFINE_SIZE] = padded_input[0..BN254_G1_RAW_AFFINE_SIZE]
                .try_into()
                .unwrap();
            let q: [u8; SCALAR_SIZE] = padded_input
                [BN254_G1_RAW_AFFINE_SIZE..BN254_G1_RAW_AFFINE_SIZE + SCALAR_SIZE]
                .try_into()
                .unwrap();

            unimplemented!()
            // let result = CryptoRuntime::bn254_mul(&mut p, &q);
            // let result = result.unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));
            // // Runtime already returns big-endian output
            // sdk.write(&result);
        }
        PRECOMPILE_BN256_PAIR => {
            let gas_used = (input.len() / BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN) as u64
                * ISTANBUL_PAIR_PER_POINT
                + ISTANBUL_PAIR_BASE;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            // validate_and_consume_gas(&sdk, gas_used, gas_limit);
            // let gas_used = required_gas;

            if input.len() % BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN != 0 {
                sdk.native_exit(ExitCode::InputOutputOutOfBounds);
            }

            let elements = input.len() / BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN;
            let mut pairs = alloc::vec::Vec::with_capacity(elements);

            for idx in 0..elements {
                // Offset to the start of the pairing element at index `idx` in the byte slice
                let start = idx * BN254_PAIRING_ELEMENT_UNCOMPRESSED_LEN;
                let g1_start = start;
                // Offset to the start of the G2 element in the pairing element
                // This is where G1 ends.
                let g2_start = start + BN254_G1_RAW_AFFINE_SIZE;

                let encoded_g1_element = &input[g1_start..g2_start];
                let encoded_g2_element = &input[g2_start..g2_start + BN254_G2_RAW_AFFINE_SIZE];

                // Get G1 and G2 points from the input
                let a = read_g1_point(encoded_g1_element)
                    .unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));
                let b = read_g2_point(encoded_g2_element)
                    .unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));

                // Always pass pairs to runtime; it will validate and skip zero pairs
                pairs.push((a, b));
            }

            // Use the runtime's REVM-compatible pairing implementation
            unimplemented!()
            // let result = CryptoRuntime::bn254_multi_pairing(&mut pairs);
            // let result = result.unwrap_or_else(|_| sdk.native_exit(ExitCode::PrecompileError));
            // sdk.sync_evm_gas(gas_used, 0);
            // sdk.write(&result);
        }
        _ => unreachable!("bn128: unsupported contract address"),
    };
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;

    fn exec_evm_precompile(address: Address, inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 200_000;
        let sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                address,
                bytecode_address: address,
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(output, expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    mod add {
        use super::*;
        #[test]
        fn test_chfast1() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f3726607c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7"),
                &hex!("2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915"),
                150,
        );
        }

        #[test]
        fn test_chfast2() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c91518b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f37266"),
                &hex!("2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb721611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb204"),
                150,
        );
        }

        #[test]
        fn test_cdetrio1() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
        );
        }

        #[test]
        fn test_cdetrio2() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
        );
        }

        #[test]
        fn test_cdetrio3() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
            );
        }

        #[test]
        fn test_cdetrio4() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!(""),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
            );
        }
        #[test]
        fn test_cdetrio5() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
            );
        }
        #[test]
        fn test_cdetrio6() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                150,
            );
        }
        #[test]
        fn test_cdetrio7() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                150,
            );
        }
        #[test]
        fn test_cdetrio8() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                150,
            );
        }
        #[test]
        fn test_cdetrio11() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002"),
                &hex!("030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd315ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4"),
                150,
            );
        }
        #[test]
        fn test_cdetrio14() {
            exec_evm_precompile(
                PRECOMPILE_BN256_ADD,
                &hex!("17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa92e83f8d734803fc370eba25ed1f6b8768bd6d83887b87165fc2434fe11a830cb00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                150,
            );
        }
    }
    mod mul {
        use super::*;
        #[test]
        fn test_chfast1() {
            exec_evm_precompile(
                PRECOMPILE_BN256_MUL,
                &hex!("2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb721611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb20400000000000000000000000000000000000000000000000011138ce750fa15c2"),
                &hex!("070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc"),
                6000,
            );
        }
        #[test]
        fn test_chfast2() {
            exec_evm_precompile(
                PRECOMPILE_BN256_MUL,
                &hex!("070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd46"),
                &hex!("025a6f4181d2b4ea8b724290ffb40156eb0adb514c688556eb79cdea0752c2bb2eff3f31dea215f1eb86023a133a996eb6300b44da664d64251d05381bb8a02e"),
                6000,
            );
        }
        #[test]
        fn test_chfast3() {
            exec_evm_precompile(
                PRECOMPILE_BN256_MUL,
                &hex!("025a6f4181d2b4ea8b724290ffb40156eb0adb514c688556eb79cdea0752c2bb2eff3f31dea215f1eb86023a133a996eb6300b44da664d64251d05381bb8a02e183227397098d014dc2822db40c0ac2ecbc0b548b438e5469e10460b6c3e7ea3"),
                &hex!("14789d0d4a730b354403b5fac948113739e276c23e0258d8596ee72f9cd9d3230af18a63153e0ec25ff9f2951dd3fa90ed0197bfef6e2a1a62b5095b9d2b4a27"),
                6000,
            );
        }
        #[test]
        fn test_cdetrio1() {
            exec_evm_precompile(
                PRECOMPILE_BN256_MUL,
                &hex!("1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f6ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                &hex!("2cde5879ba6f13c0b5aa4ef627f159a3347df9722efce88a9afbb20b763b4c411aa7e43076f6aee272755a7f9b84832e71559ba0d2e0b17d5f9f01755e5b0d11"),
                6000,
            );
        }
        #[test]
        fn test_ccdetrio2() {
            exec_evm_precompile(
                PRECOMPILE_BN256_MUL,
                &hex!("1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f630644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000000"),
                &hex!("1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe3163511ddc1c3f25d396745388200081287b3fd1472d8339d5fecb2eae0830451"),
                6000,
            );
        }
    }
    mod pair {
        use super::*;
        #[test]
        fn test_jeff1() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                113000,
            );
        }
        #[test]
        fn test_jeff2() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("2eca0c7238bf16e83e7a1e6c5d49540685ff51380f309842a98561558019fc0203d3260361bb8451de5ff5ecd17f010ff22f5c31cdf184e9020b06fa5997db841213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f06967a1237ebfeca9aaae0d6d0bab8e28c198c5a339ef8a2407e31cdac516db922160fa257a5fd5b280642ff47b65eca77e626cb685c84fa6d3b6882a283ddd1198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                113000,
            );
        }
        #[test]
        fn test_jeff3() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("0f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd216da2f5cb6be7a0aa72c440c53c9bbdfec6c36c7d515536431b3a865468acbba2e89718ad33c8bed92e210e81d1853435399a271913a6520736a4729cf0d51eb01a9e2ffa2e92599b68e44de5bcf354fa2642bd4f26b259daa6f7ce3ed57aeb314a9a87b789a58af499b314e13c3d65bede56c07ea2d418d6874857b70763713178fb49a2d6cd347dc58973ff49613a20757d0fcc22079f9abd10c3baee245901b9e027bd5cfc2cb5db82d4dc9677ac795ec500ecd47deee3b5da006d6d049b811d7511c78158de484232fc68daf8a45cf217d1c2fae693ff5871e8752d73b21198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                113000,
            );
        }
        #[test]
        fn test_jeff4() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("2f2ea0b3da1e8ef11914acf8b2e1b32d99df51f5f4f206fc6b947eae860eddb6068134ddb33dc888ef446b648d72338684d678d2eb2371c61a50734d78da4b7225f83c8b6ab9de74e7da488ef02645c5a16a6652c3c71a15dc37fe3a5dcb7cb122acdedd6308e3bb230d226d16a105295f523a8a02bfc5e8bd2da135ac4c245d065bbad92e7c4e31bf3757f1fe7362a63fbfee50e7dc68da116e67d600d9bf6806d302580dc0661002994e7cd3a7f224e7ddc27802777486bf80f40e4ca3cfdb186bac5188a98c45e6016873d107f5cd131f3a3e339d0375e58bd6219347b008122ae2b09e539e152ec5364e7e2204b03d11d3caa038bfc7cd499f8176aacbee1f39e4e4afc4bc74790a4a028aff2c3d2538731fb755edefd8cb48d6ea589b5e283f150794b6736f670d6a1033f9b46c6f5204f50813eb85c8dc4b59db1c5d39140d97ee4d2b36d99bc49974d18ecca3e7ad51011956051b464d9e27d46cc25e0764bb98575bd466d32db7b15f582b2d5c452b36aa394b789366e5e3ca5aabd415794ab061441e51d01e94640b7e3084a07e02c78cf3103c542bc5b298669f211b88da1679b0b64a63b7e0e7bfe52aae524f73a55be7fe70c7e9bfc94b4cf0da1213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                147000,
            );
        }
        #[test]
        fn test_jeff5() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("20a754d2071d4d53903e3b31a7e98ad6882d58aec240ef981fdf0a9d22c5926a29c853fcea789887315916bbeb89ca37edb355b4f980c9a12a94f30deeed30211213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f1abb4a25eb9379ae96c84fff9f0540abcfc0a0d11aeda02d4f37e4baf74cb0c11073b3ff2cdbb38755f8691ea59e9606696b3ff278acfc098fa8226470d03869217cee0a9ad79a4493b5253e2e4e3a39fc2df38419f230d341f60cb064a0ac290a3d76f140db8418ba512272381446eb73958670f00cf46f1d9e64cba057b53c26f64a8ec70387a13e41430ed3ee4a7db2059cc5fc13c067194bcc0cb49a98552fd72bd9edb657346127da132e5b82ab908f5816c826acb499e22f2412d1a2d70f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd2198a1f162a73261f112401aa2db79c7dab1533c9935c77290a6ce3b191f2318d198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                147000,
            );
        }
        #[test]
        fn test_jeff6() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!("1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c103188585e2364128fe25c70558f1560f4f9350baf3959e603cc91486e110936198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000000"),
                113000,
            );
        }
        #[test]
        fn test_empty_data() {
            exec_evm_precompile(
                PRECOMPILE_BN256_PAIR,
                &hex!(""),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                45000,
            );
        }
    }
}
