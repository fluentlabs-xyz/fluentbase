#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;

use fluentbase_sdk::{
    alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI,
    PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
};

// G1_ADD_BASE_GAS_FEE: u64 = 375;
// G1_MSM_BASE_GAS_FEE: u64 = 12000;

// G2_ADD_BASE_GAS_FEE: u64 = 600;
// G2_MSM_BASE_GAS_FEE: u64 = 22500;

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
fn bls12_381_g2_add_with_sdk<SDK: SharedAPI>(_: &SDK, p: &mut [u8; 192], q: &[u8; 192]) {
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
            // EIP-2537: input must be 512 bytes (two G2 elements, each 256 bytes padded)
            if input.len() != 512 {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            // Split inputs: each G2 is x0||x1||y0||y1 (each 64-byte padded BE, 48-byte value)
            let a = &input[0..256];
            let b = &input[256..512];
            let (a_x0, a_x1, a_y0, a_y1) = (&a[0..64], &a[64..128], &a[128..192], &a[192..256]);
            let (b_x0, b_x1, b_y0, b_y1) = (&b[0..64], &b[64..128], &b[128..192], &b[192..256]);

            // Convert to runtime format: 192 bytes LE (x0||x1||y0||y1), each limb 48 bytes
            let mut p = [0u8; 192];
            let mut q = [0u8; 192];
            // a.x0
            p[0..48].copy_from_slice(&a_x0[16..64]);
            p[0..48].reverse();
            // a.x1
            p[48..96].copy_from_slice(&a_x1[16..64]);
            p[48..96].reverse();
            // a.y0
            p[96..144].copy_from_slice(&a_y0[16..64]);
            p[96..144].reverse();
            // a.y1
            p[144..192].copy_from_slice(&a_y1[16..64]);
            p[144..192].reverse();

            // b.x0
            q[0..48].copy_from_slice(&b_x0[16..64]);
            q[0..48].reverse();
            // b.x1
            q[48..96].copy_from_slice(&b_x1[16..64]);
            q[48..96].reverse();
            // b.y0
            q[96..144].copy_from_slice(&b_y0[16..64]);
            q[96..144].reverse();
            // b.y1
            q[144..192].copy_from_slice(&b_y1[16..64]);
            q[144..192].reverse();

            bls12_381_g2_add_with_sdk(&sdk, &mut p, &q);
            let gas_used = 600u64;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);

            // Encode output: 256 bytes (x0||x1||y0||y1), each limb is 64-byte BE padded (16 zeros + 48 value)
            let mut out = [0u8; 256];
            let mut limb = [0u8; 48];
            // x0
            limb.copy_from_slice(&p[0..48]);
            limb.reverse();
            out[16..64].copy_from_slice(&limb);
            // x1
            limb.copy_from_slice(&p[48..96]);
            limb.reverse();
            out[80..128].copy_from_slice(&limb);
            // y0
            limb.copy_from_slice(&p[96..144]);
            limb.reverse();
            out[144..192].copy_from_slice(&limb);
            // y1
            limb.copy_from_slice(&p[144..192]);
            limb.reverse();
            out[208..256].copy_from_slice(&limb);

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

    // ==================================== G1 ADD ====================================
    #[test]
    fn bls_g1add_g1_g1_2_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_ADD,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1"),
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28"),
            375,
        );
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
    #[test]
    fn bls_g1add_inf_g1_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_ADD,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1"),
            375,
        );
    }
    // ==================================== G2 ADD ====================================
    #[test]
    fn bls_g2add_g2_g2_2_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_ADD,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"),
            &hex!("000000000000000000000000000000001638533957d540a9d2370f17cc7ed5863bc0b995b8825e0ee1ea1e1e4d00dbae81f14b0bf3611b78c952aacab827a053000000000000000000000000000000000a4edef9c1ed7f729f520e47730a124fd70662a904ba1074728114d1031e1572c6c886f6b57ec72a6178288c47c33577000000000000000000000000000000000468fb440d82b0630aeb8dca2b5256789a66da69bf91009cbfe6bd221e47aa8ae88dece9764bf3bd999d95d71e4c9899000000000000000000000000000000000f6d4552fa65dd2638b361543f887136a43253d9c66c411697003f7a13c308f5422e1aa0a59c8967acdefd8b6e36ccf3"),
            600,
        );
    }
    #[test]
    fn bls_g2add_2_g2_3_g2_5_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_ADD,
            &hex!("000000000000000000000000000000001638533957d540a9d2370f17cc7ed5863bc0b995b8825e0ee1ea1e1e4d00dbae81f14b0bf3611b78c952aacab827a053000000000000000000000000000000000a4edef9c1ed7f729f520e47730a124fd70662a904ba1074728114d1031e1572c6c886f6b57ec72a6178288c47c33577000000000000000000000000000000000468fb440d82b0630aeb8dca2b5256789a66da69bf91009cbfe6bd221e47aa8ae88dece9764bf3bd999d95d71e4c9899000000000000000000000000000000000f6d4552fa65dd2638b361543f887136a43253d9c66c411697003f7a13c308f5422e1aa0a59c8967acdefd8b6e36ccf300000000000000000000000000000000122915c824a0857e2ee414a3dccb23ae691ae54329781315a0c75df1c04d6d7a50a030fc866f09d516020ef82324afae0000000000000000000000000000000009380275bbc8e5dcea7dc4dd7e0550ff2ac480905396eda55062650f8d251c96eb480673937cc6d9d6a44aaa56ca66dc000000000000000000000000000000000b21da7955969e61010c7a1abc1a6f0136961d1e3b20b1a7326ac738fef5c721479dfd948b52fdf2455e44813ecfd8920000000000000000000000000000000008f239ba329b3967fe48d718a36cfe5f62a7e42e0bf1c1ed714150a166bfbd6bcf6b3b58b975b9edea56d53f23a0e849"),
            &hex!("000000000000000000000000000000000411a5de6730ffece671a9f21d65028cc0f1102378de124562cb1ff49db6f004fcd14d683024b0548eff3d1468df26880000000000000000000000000000000000fb837804dba8213329db46608b6c121d973363c1234a86dd183baff112709cf97096c5e9a1a770ee9d7dc641a894d60000000000000000000000000000000019b5e8f5d4a72f2b75811ac084a7f814317360bac52f6aab15eed416b4ef9938e0bdc4865cc2c4d0fd947e7c6925fd1400000000000000000000000000000000093567b4228be17ee62d11a254edd041ee4b953bffb8b8c7f925bd6662b4298bac2822b446f5b5de3b893e1be5aa4986"),
            600,
        );
    }

    // ==================================== G2 MSM ====================================
}
