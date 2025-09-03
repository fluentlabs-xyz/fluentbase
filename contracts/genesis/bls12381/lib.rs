#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI,
    PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
};

#[inline(always)]
fn array_ref64(bytes: &[u8], offset: usize) -> &[u8; 64] {
    // Safety: caller ensures bounds and alignment for 64 bytes slice
    let slice = &bytes[offset..offset + 64];
    unsafe { &*(slice.as_ptr() as *const [u8; 64]) }
}

#[inline(always)]
fn bls12_381_g1_add_with_sdk<SDK: SharedAPI>(_: &SDK, p: &mut [u8; 96], q: &[u8; 96]) {
    SDK::bls12_381_g1_add(p, q)
}
#[inline(always)]
fn bls12_381_g2_add_with_sdk<SDK: SharedAPI>(_: &SDK, p: &mut [u8; 64], q: &[u8; 64]) {
    SDK::bls12_381_g2_add(p, q)
}
#[inline(always)]
fn bls12_381_g1_msm_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 64], [u8; 64])],
    out: &mut [u8; 64],
) {
    SDK::bls12_381_g1_msm(pairs, out)
}
#[inline(always)]
fn bls12_381_g2_msm_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 64], [u8; 64])],
    out: &mut [u8; 64],
) {
    SDK::bls12_381_g2_msm(pairs, out)
}
#[inline(always)]
fn bls12_381_pairing_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 64], [u8; 64])],
    out: &mut [u8; 64],
) {
    SDK::bls12_381_pairing(pairs, out)
}
#[inline(always)]
fn bls12_381_map_fp_to_g1_with_sdk<SDK: SharedAPI>(_: &SDK, p: &[u8; 64], out: &mut [u8; 64]) {
    SDK::bls12_381_map_fp_to_g1(p, out)
}
#[inline(always)]
fn bls12_381_map_fp2_to_g2_with_sdk<SDK: SharedAPI>(_: &SDK, p: &[u8; 64], out: &mut [u8; 64]) {
    SDK::bls12_381_map_fp2_to_g2(p, out)
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    // read full input data
    let bytecode_address = sdk.context().contract_bytecode_address();
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let mut input = alloc_slice(input_length as usize);
    sdk.read(&mut input, 0);
    let input = Bytes::copy_from_slice(input);
    // dispatch to SDK-backed implementation
    match bytecode_address {
        PRECOMPILE_BLS12_381_G1_ADD => {
            // Expect two G1 points (x1||y1||x2||y2), each coord 64 bytes BE padded
            if input.len() != 256 {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            // Split inputs
            let x1_be = &input[0..64];
            let y1_be = &input[64..128];
            let x2_be = &input[128..192];
            let y2_be = &input[192..256];

            // Convert to runtime format: 96 bytes LE (x48||y48)
            let mut p = [0u8; 96];
            let mut q = [0u8; 96];
            // p.x: take 48-byte field from x1_be[16..64] and reverse to LE
            p[0..48].copy_from_slice(&x1_be[16..64]);
            p[0..48].reverse();
            // p.y
            p[48..96].copy_from_slice(&y1_be[16..64]);
            p[48..96].reverse();
            // q.x
            q[0..48].copy_from_slice(&x2_be[16..64]);
            q[0..48].reverse();
            // q.y
            q[48..96].copy_from_slice(&y2_be[16..64]);
            q[48..96].reverse();

            bls12_381_g1_add_with_sdk(&sdk, &mut p, &q);
            let gas_used = 375u64;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            // EVM expects X||Y, each 64 bytes BE, where the 48-byte field is left-padded
            let mut out = [0u8; 128];
            // x: take 48 LE -> BE and place at [16..64]
            let mut x_be48 = [0u8; 48];
            x_be48.copy_from_slice(&p[0..48]);
            x_be48.reverse();
            out[16..64].copy_from_slice(&x_be48);
            // y: [48..96] LE -> BE -> [80..128]
            let mut y_be48 = [0u8; 48];
            y_be48.copy_from_slice(&p[48..96]);
            y_be48.reverse();
            out[80..128].copy_from_slice(&y_be48);
            sdk.write(&out);
        }
        PRECOMPILE_BLS12_381_G2_ADD => {
            if input.len() < 128 {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let start = input.len() - 128;
            let mut p = [0u8; 64];
            let mut q = [0u8; 64];
            p.copy_from_slice(&input[start..start + 64]);
            q.copy_from_slice(&input[start + 64..start + 128]);
            // Convert BE -> LE before calling runtime, then back to BE for output
            p[0..32].reverse();
            p[32..64].reverse();
            let mut q_conv = q;
            q_conv[0..32].reverse();
            q_conv[32..64].reverse();
            bls12_381_g2_add_with_sdk(&sdk, &mut p, &q_conv);
            let gas_used = 375u64;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            let mut out = [0u8; 128];
            let mut x_be = [0u8; 32];
            x_be.copy_from_slice(&p[0..32]);
            x_be.reverse();
            let mut y_be = [0u8; 32];
            y_be.copy_from_slice(&p[32..64]);
            y_be.reverse();
            out[32..64].copy_from_slice(&x_be);
            out[96..128].copy_from_slice(&y_be);
            sdk.write(&out);
        }
        PRECOMPILE_BLS12_381_G1_MSM => {
            if input.len() % 128 != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / 128;
            let mut pairs: alloc::vec::Vec<([u8; 64], [u8; 64])> =
                alloc::vec::Vec::with_capacity(pairs_len);
            for i in 0..pairs_len {
                let mut a = [0u8; 64];
                let mut b = [0u8; 64];
                let start = i * 128;
                a.copy_from_slice(&input[start..start + 64]);
                b.copy_from_slice(&input[start + 64..start + 128]);
                a[0..32].reverse();
                a[32..64].reverse();
                b[0..32].reverse();
                b[32..64].reverse();
                pairs.push((a, b));
            }
            let mut out = [0u8; 64];
            bls12_381_g1_msm_with_sdk(&sdk, &pairs, &mut out);
            let gas_used = 250u64.saturating_mul(pairs_len as u64).saturating_add(100);
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            let mut x_be = [0u8; 32];
            x_be.copy_from_slice(&out[0..32]);
            x_be.reverse();
            let mut y_be = [0u8; 32];
            y_be.copy_from_slice(&out[32..64]);
            y_be.reverse();
            let mut out_be = [0u8; 64];
            out_be[0..32].copy_from_slice(&x_be);
            out_be[32..64].copy_from_slice(&y_be);
            sdk.write(&out_be);
        }
        PRECOMPILE_BLS12_381_G2_MSM => {
            if input.len() % 128 != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / 128;
            let mut pairs: alloc::vec::Vec<([u8; 64], [u8; 64])> =
                alloc::vec::Vec::with_capacity(pairs_len);
            for i in 0..pairs_len {
                let mut a = [0u8; 64];
                let mut b = [0u8; 64];
                let start = i * 128;
                a.copy_from_slice(&input[start..start + 64]);
                b.copy_from_slice(&input[start + 64..start + 128]);
                a[0..32].reverse();
                a[32..64].reverse();
                b[0..32].reverse();
                b[32..64].reverse();
                pairs.push((a, b));
            }
            let mut out = [0u8; 64];
            bls12_381_g2_msm_with_sdk(&sdk, &pairs, &mut out);
            let gas_used = 300u64.saturating_mul(pairs_len as u64).saturating_add(100);
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            let mut x_be = [0u8; 32];
            x_be.copy_from_slice(&out[0..32]);
            x_be.reverse();
            let mut y_be = [0u8; 32];
            y_be.copy_from_slice(&out[32..64]);
            y_be.reverse();
            let mut out_be = [0u8; 64];
            out_be[0..32].copy_from_slice(&x_be);
            out_be[32..64].copy_from_slice(&y_be);
            sdk.write(&out_be);
        }
        PRECOMPILE_BLS12_381_PAIRING => {
            if input.len() % 128 != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / 128;
            let mut pairs: alloc::vec::Vec<([u8; 64], [u8; 64])> =
                alloc::vec::Vec::with_capacity(pairs_len);
            for i in 0..pairs_len {
                let mut g1 = [0u8; 64];
                let mut g2 = [0u8; 64];
                let start = i * 128;
                g1.copy_from_slice(&input[start..start + 64]);
                g2.copy_from_slice(&input[start + 64..start + 128]);
                g1[0..32].reverse();
                g1[32..64].reverse();
                g2[0..32].reverse();
                g2[32..64].reverse();
                pairs.push((g1, g2));
            }
            let mut out = [0u8; 64];
            bls12_381_pairing_with_sdk(&sdk, &pairs, &mut out);
            let gas_used = 400u64.saturating_mul(pairs_len as u64).saturating_add(100);
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            let mut x_be = [0u8; 32];
            x_be.copy_from_slice(&out[0..32]);
            x_be.reverse();
            let mut y_be = [0u8; 32];
            y_be.copy_from_slice(&out[32..64]);
            y_be.reverse();
            let mut out_be = [0u8; 64];
            out_be[0..32].copy_from_slice(&x_be);
            out_be[32..64].copy_from_slice(&y_be);
            sdk.write(&out_be);
        }
        PRECOMPILE_BLS12_381_MAP_G1 => {
            if input.len() != 64 {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let mut p = [0u8; 64];
            bls12_381_map_fp_to_g1_with_sdk(&sdk, array_ref64(&input, 0), &mut p);
            let gas_used = 250u64;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            sdk.write(&p);
        }
        PRECOMPILE_BLS12_381_MAP_G2 => {
            if input.len() != 64 {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let mut p = [0u8; 64];
            bls12_381_map_fp2_to_g2_with_sdk(&sdk, array_ref64(&input, 0), &mut p);
            let gas_used = 250u64;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            sdk.write(&p);
        }
        _ => unreachable!("bls12381: unsupported contract address"),
    }
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_sdk_testing::HostTestingContext;

    fn exec_evm_precompile(address: Address, inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 100_000;
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

    #[test]
    fn bls_g1add_2_g1_3_g1_5_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_ADD,
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d280000000000000000000000000000000009ece308f9d1f0131765212deca99697b112d61f9be9a5f1f3780a51335b3ff981747a0b2ca2179b96d2c0c9024e522400000000000000000000000000000000032b80d3a6f5b09f8a84623389c5f80ca69a0cddabc3097f9d9c27310fd43be6e745256c634af45ca3473b0590ae30d1"),
            &hex!("0000000000000000000000000000000010e7791fb972fe014159aa33a98622da3cdc98ff707965e536d8636b5fcc5ac7a91a8c46e59a00dca575af0f18fb13dc0000000000000000000000000000000016ba437edcc6551e30c10512367494bfb6b01cc6681e8a4c3cd2501832ab5c4abc40b4578b85cbaffbf0bcd70d67c6e2"),
            375,
        );
    }
}
