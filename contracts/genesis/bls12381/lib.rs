#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;

use fluentbase_sdk::{
    alloc_slice, entrypoint, Bytes, ContextReader, ExitCode, SharedAPI,
    PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
};

pub const G1_ADD_GAS: u64 = 375u64;
pub const G2_ADD_GAS: u64 = 600u64;
pub const G1_MSM_GAS: u64 = 12000u64;
pub const G2_MSM_GAS: u64 = 22500u64;
pub const PAIRING_GAS: u64 = 288u64;
pub const MAP_G1_GAS: u64 = 250u64;
pub const MAP_G2_GAS: u64 = 250u64;

/// SCALAR_LENGTH specifies the number of bytes needed to represent a scalar.
///
/// Note: The scalar is represented in little endian.
pub const SCALAR_LENGTH: usize = 32;

pub const FP_LENGTH: usize = 48;
/// PADDED_FP_LENGTH specifies the number of bytes that the EVM will use
/// to represent an Fp element according to EIP-2537.
///
/// Note: We only need FP_LENGTH number of bytes to represent it,
/// but we pad the byte representation to be 32 byte aligned as specified in EIP 2537.
pub const PADDED_FP_LENGTH: usize = 64;

/// G1_LENGTH specifies the number of bytes needed to represent a G1 element.
///
/// Note: A G1 element contains 2 Fp elements.
pub const G1_LENGTH: usize = 2 * FP_LENGTH;
/// PADDED_G1_LENGTH specifies the number of bytes that the EVM will use to represent
/// a G1 element according to padding rules specified in EIP-2537.
pub const PADDED_G1_LENGTH: usize = 2 * PADDED_FP_LENGTH;

/// G1_ADD_INPUT_LENGTH specifies the number of bytes that the input to G1ADD
/// must use.
///
/// Note: The input to the G1 addition precompile is 2 G1 elements.
pub const G1_ADD_INPUT_LENGTH: usize = 2 * PADDED_G1_LENGTH;

/// FP2_LENGTH specifies the number of bytes needed to represent a Fp^2 element.
///
/// Note: This is the quadratic extension of Fp, and by definition
/// means we need 2 Fp elements.
pub const FP2_LENGTH: usize = 2 * FP_LENGTH;

/// G2_LENGTH specifies the number of bytes needed to represent a G2 element.
///
/// Note: A G2 element contains 2 Fp^2 elements.
pub const G2_LENGTH: usize = 2 * FP2_LENGTH;

/// PADDED_FP2_LENGTH specifies the number of bytes that the EVM will use to represent
/// a Fp^2 element according to the padding rules specified in EIP-2537.
///
/// Note: This is the quadratic extension of Fp, and by definition
/// means we need 2 Fp elements.
pub const PADDED_FP2_LENGTH: usize = 2 * PADDED_FP_LENGTH;

/// PADDED_G2_LENGTH specifies the number of bytes that the EVM will use to represent
/// a G2 element.
///
/// Note: A G2 element can be represented using 2 Fp^2 elements.
pub const PADDED_G2_LENGTH: usize = 2 * PADDED_FP2_LENGTH;

/// G2_ADD_INPUT_LENGTH specifies the number of bytes that the input to G2ADD
/// must occupy.
///
/// Note: The input to the G2 addition precompile is 2 G2 elements.
pub const G2_ADD_INPUT_LENGTH: usize = 2 * PADDED_G2_LENGTH;

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
fn bls12_381_g2_add_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    p: &mut [u8; 192],
    q: &[u8; 192],
) -> [u8; 192] {
    SDK::bls12_381_g2_add(p, q)
}
#[inline(always)]
fn bls12_381_g1_msm_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 96], [u8; 32])],
    out: &mut [u8; 96],
) {
    SDK::bls12_381_g1_msm(pairs, out)
}
#[inline(always)]
fn bls12_381_g2_msm_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 192], [u8; 32])],
    out: &mut [u8; 192],
) {
    SDK::bls12_381_g2_msm(pairs, out)
}
#[inline(always)]
fn bls12_381_pairing_with_sdk<SDK: SharedAPI>(
    _: &SDK,
    pairs: &[([u8; 48], [u8; 96])],
    out: &mut [u8; 288],
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
            let gas_used = G1_ADD_GAS;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);

            // Expect two G1 points (x1||y1||x2||y2), each coord 64 bytes BE padded
            if input_length != G1_ADD_INPUT_LENGTH as u32 {
                sdk.native_exit(ExitCode::PrecompileError);
            }

            // Split into two 128-byte points
            let a = &input[0..128];
            let b = &input[128..256];

            // a = x1||y1, b = x2||y2 (each limb 64B BE padded, 48B value)
            let (x1_be, y1_be) = (&a[0..64], &a[64..128]);
            let (x2_be, y2_be) = (&b[0..64], &b[64..128]);

            // Convert to runtime format: 96 bytes BE (x48||y48) as expected by blstrs::G1Affine::from_uncompressed
            let mut p = [0u8; 96];
            let mut q = [0u8; 96];
            // p.x
            p[0..48].copy_from_slice(&x1_be[16..64]);
            // p.y
            p[48..96].copy_from_slice(&y1_be[16..64]);
            // q.x
            q[0..48].copy_from_slice(&x2_be[16..64]);
            // q.y
            q[48..96].copy_from_slice(&y2_be[16..64]);

            bls12_381_g1_add_with_sdk(&sdk, &mut p, &q);

            // EVM expects X||Y, each 64 bytes BE, where the 48-byte field is left-padded
            let mut out = [0u8; 128];
            // x: 48 LE -> BE and place at [16..64]
            out[16..64].copy_from_slice(&p[0..48]);
            // y: 48 LE -> BE and place at [80..128]
            out[80..128].copy_from_slice(&p[48..96]);
            sdk.write(&out);
        }
        PRECOMPILE_BLS12_381_G2_ADD => {
            let gas_used = G2_ADD_GAS;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);

            // EIP-2537: input must be 512 bytes (two G2 elements, each 256 bytes padded)
            if input_length != G2_ADD_INPUT_LENGTH as u32 {
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
            if input.len() % (PADDED_G1_LENGTH + 32) != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / (PADDED_G1_LENGTH + 32);
            let mut pairs: alloc::vec::Vec<([u8; 96], [u8; 32])> =
                alloc::vec::Vec::with_capacity(pairs_len);
            let gas_used = G1_MSM_GAS;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            for i in 0..pairs_len {
                let start = i * (PADDED_G1_LENGTH + 32);
                let g1_in = &input[start..start + PADDED_G1_LENGTH];
                let s_be = &input[start + PADDED_G1_LENGTH..start + PADDED_G1_LENGTH + 32];
                let mut p = [0u8; 96];
                p[0..48].copy_from_slice(&g1_in[16..64]);
                p[48..96].copy_from_slice(&g1_in[80..128]);
                let mut s_le = [0u8; 32];
                for j in 0..32 {
                    s_le[j] = s_be[31 - j];
                }
                pairs.push((p, s_le));
            }
            let mut out96 = [0u8; 96];
            bls12_381_g1_msm_with_sdk(&sdk, &pairs, &mut out96);
            // Detect identity (blstrs sets flag bit for infinity in first byte of uncompressed)
            if out96[0] & 0x40 != 0 {
                let out = [0u8; 128];
                sdk.write(&out);
            } else {
                let out = {
                    let mut tmp = [0u8; 128];
                    tmp[16..64].copy_from_slice(&out96[0..48]);
                    tmp[80..128].copy_from_slice(&out96[48..96]);
                    tmp
                };
                sdk.write(&out);
            }
        }
        PRECOMPILE_BLS12_381_G2_MSM => {
            // Expect pairs of 288 bytes: 256-byte padded G2 point (x0||x1||y0||y1) + 32-byte scalar (BE)
            // Convert to runtime format: 192-byte LE limbs + 32-byte scalar LE
            let input_length_requirement = PADDED_G2_LENGTH + 32; // 256 + 32

            if input.len() % input_length_requirement != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / input_length_requirement;
            let mut pairs: alloc::vec::Vec<([u8; 192], [u8; 32])> =
                alloc::vec::Vec::with_capacity(pairs_len);

            let gas_used = G2_MSM_GAS;
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }

            sdk.sync_evm_gas(gas_used, 0);
            for i in 0..pairs_len {
                let mut p = [0u8; 192];
                let mut s = [0u8; SCALAR_LENGTH];
                let start = i * input_length_requirement;
                let g2_in = &input[start..start + PADDED_G2_LENGTH];

                // Convert padded BE limbs → LE limbs (like G2 add path)
                let mut limb = [0u8; 48];
                // x0
                limb.copy_from_slice(&g2_in[0..64][16..64]);
                limb.reverse();
                p[0..48].copy_from_slice(&limb);
                // x1
                limb.copy_from_slice(&g2_in[64..128][16..64]);
                limb.reverse();
                p[48..96].copy_from_slice(&limb);
                // y0
                limb.copy_from_slice(&g2_in[128..192][16..64]);
                limb.reverse();
                p[96..144].copy_from_slice(&limb);
                // y1
                limb.copy_from_slice(&g2_in[192..256][16..64]);
                limb.reverse();
                p[144..192].copy_from_slice(&limb);

                // Scalar: 32B BE → 32B LE
                s.copy_from_slice(
                    &input[start + PADDED_G2_LENGTH..start + PADDED_G2_LENGTH + SCALAR_LENGTH],
                );
                s.reverse();

                pairs.push((p, s));
            }
            let mut out = [0u8; 192];
            bls12_381_g2_msm_with_sdk(&sdk, &pairs, &mut out);
            // Encode output to 256B padded BE like G2 add path
            if out.iter().all(|&b| b == 0) {
                let out_be = [0u8; 256];
                sdk.write(&out_be);
            } else {
                let mut out_be = [0u8; 256];
                let mut limb = [0u8; 48];
                // x0
                limb.copy_from_slice(&out[0..48]);
                limb.reverse();
                out_be[16..64].copy_from_slice(&limb);
                // x1
                limb.copy_from_slice(&out[48..96]);
                limb.reverse();
                out_be[80..128].copy_from_slice(&limb);
                // y0
                limb.copy_from_slice(&out[96..144]);
                limb.reverse();
                out_be[144..192].copy_from_slice(&limb);
                // y1
                limb.copy_from_slice(&out[144..192]);
                limb.reverse();
                out_be[208..256].copy_from_slice(&limb);
                sdk.write(&out_be);
            }
        }
        PRECOMPILE_BLS12_381_PAIRING => {
            if input.len() % 128 != 0 || input.is_empty() {
                sdk.native_exit(ExitCode::PrecompileError);
            }
            let pairs_len = input.len() / 128;
            let mut pairs: alloc::vec::Vec<([u8; 48], [u8; 96])> =
                alloc::vec::Vec::with_capacity(pairs_len);
            for i in 0..pairs_len {
                let mut g1 = [0u8; 48];
                let mut g2 = [0u8; 96];
                let start = i * 128;
                // Parse G1: x||y (each 32-byte BE padded, 48-byte value)
                // Extract 48B limbs (skip leading 16 zero bytes per limb) and convert to LE
                g1[0..48].copy_from_slice(&input[start..start + 64][16..64]);
                g1[0..48].reverse();
                // Parse G2: x0||x1||y0||y1, each limb 64B BE padded
                let g2_in = &input[start + 64..start + 128];
                // x0: 32B BE -> 48B LE (zero-extended)
                let mut limb = [0u8; 48];
                limb[0..32].copy_from_slice(&g2_in[0..32]);
                limb[0..32].reverse();
                g2[0..48].copy_from_slice(&limb);
                // x1: 32B BE -> 48B LE (zero-extended)
                limb[0..32].copy_from_slice(&g2_in[32..64]);
                limb[0..32].reverse();
                limb[32..48].fill(0);
                g2[48..96].copy_from_slice(&limb);
                pairs.push((g1, g2));
            }
            let mut out = [0u8; 288];
            bls12_381_pairing_with_sdk(&sdk, &pairs, &mut out);
            let gas_used = 400u64.saturating_mul(pairs_len as u64).saturating_add(100);
            if gas_used > gas_limit {
                sdk.native_exit(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(gas_used, 0);
            // Decode compressed GT and return EIP-197 boolean (32-byte BE 0/1)
            let is_one = {
                // Compare against compressed identity directly to avoid extra deps
                let zero = [0u8; 288];
                // blstrs writes six Fp limbs (each 48B LE). For identity, compression output is zero.
                // A zero buffer is a valid identity compression.
                out == zero
            };
            let mut out_be = [0u8; 32];
            if is_one {
                out_be[31] = 1;
            }
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
    // ==================================== G1 MSM ====================================
    #[test]
    fn bls_g1mul_g1_add_g1_2_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28"),
            12000,
        );
    }
    #[test]
    fn bls_g1mul_p1_add_p1_2_p1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("0000000000000000000000000000000015222cddbabdd764c4bee0b3720322a65ff4712c86fc4b1588d0c209210a0884fa9468e855d261c483091b2bf7de6a630000000000000000000000000000000009f9edb99bc3b75d7489735c98b16ab78b9386c5f7a1f76c7e96ac6eb5bbde30dbca31a74ec6e0f0b12229eecea33c39"),
            12000,
        );
    }
    #[test]
    fn bls_g1mul_1_mul_g1_g1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1"),
            12000,
        );
    }
    #[test]
    fn bls_g1mul_1_mul_p1_p1() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21"),
            12000,
        );
    }
    #[test]
    fn bls_g1mul_0_mul_g1_inf() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            12000,
        );
    }
    #[test]
    fn bls_g1mul_0_mul_p1_inf() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            12000,
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
    #[test]
    fn bls_g2add_inf_g2_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_ADD,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"),
            600,
        );
    }
    // ==================================== G2 MSM ====================================
    #[test]
    fn bls_g2mul_g2_add_g2_2_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("000000000000000000000000000000001638533957d540a9d2370f17cc7ed5863bc0b995b8825e0ee1ea1e1e4d00dbae81f14b0bf3611b78c952aacab827a053000000000000000000000000000000000a4edef9c1ed7f729f520e47730a124fd70662a904ba1074728114d1031e1572c6c886f6b57ec72a6178288c47c33577000000000000000000000000000000000468fb440d82b0630aeb8dca2b5256789a66da69bf91009cbfe6bd221e47aa8ae88dece9764bf3bd999d95d71e4c9899000000000000000000000000000000000f6d4552fa65dd2638b361543f887136a43253d9c66c411697003f7a13c308f5422e1aa0a59c8967acdefd8b6e36ccf3"),
            22500,
        );
    }
    #[test]
    fn bls_g2mul_p2_add_p2_2_p2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d8784510000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("000000000000000000000000000000000b76fcbb604082a4f2d19858a7befd6053fa181c5119a612dfec83832537f644e02454f2b70d40985ebb08042d1620d40000000000000000000000000000000019a4a02c0ae51365d964c73be7babb719db1c69e0ddbf9a8a335b5bed3b0a4b070d2d5df01d2da4a3f1e56aae2ec106d000000000000000000000000000000000d18322f821ac72d3ca92f92b000483cf5b7d9e5d06873a44071c4e7e81efd904f210208fe0b9b4824f01c65bc7e62080000000000000000000000000000000004e563d53609a2d1e216aaaee5fbc14ef460160db8d1fdc5e1bd4e8b54cd2f39abf6f925969fa405efb9e700b01c7085"),
            22500,
        );
    }
    #[test]
    fn bls_g2mul_1_mul_g2_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"),
            22500,
        );
    }
    #[test]
    fn bls_g2mul_1_mul_p2_p2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d8784510000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"),
            22500,
        );
    }
    #[test]
    fn bls_g2mul_0_mul_g2_inf() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            22500,
        );
    }
    #[test]
    fn bls_g2mul_0_mul_p2_inf() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d8784510000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            22500,
        );
    }
    // ==================================== Pairing ====================================

    // bls_pairing_e(2*G1,3*G2)=e(5*G1,G2)
    #[test]
    fn bls_pairing_e2_g1_3_g2_e_5_g1_g2() {
        exec_evm_precompile(
            PRECOMPILE_BLS12_381_PAIRING,
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d2800000000000000000000000000000000122915c824a0857e2ee414a3dccb23ae691ae54329781315a0c75df1c04d6d7a50a030fc866f09d516020ef82324afae0000000000000000000000000000000009380275bbc8e5dcea7dc4dd7e0550ff2ac480905396eda55062650f8d251c96eb480673937cc6d9d6a44aaa56ca66dc000000000000000000000000000000000b21da7955969e61010c7a1abc1a6f0136961d1e3b20b1a7326ac738fef5c721479dfd948b52fdf2455e44813ecfd8920000000000000000000000000000000008f239ba329b3967fe48d718a36cfe5f62a7e42e0bf1c1ed714150a166bfbd6bcf6b3b58b975b9edea56d53f23a0e8490000000000000000000000000000000010e7791fb972fe014159aa33a98622da3cdc98ff707965e536d8636b5fcc5ac7a91a8c46e59a00dca575af0f18fb13dc0000000000000000000000000000000016ba437edcc6551e30c10512367494bfb6b01cc6681e8a4c3cd2501832ab5c4abc40b4578b85cbaffbf0bcd70d67c6e200000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000"),
            600,
        );
    }

    // bls_pairing_e(2*G1,3*G2)=e(6*G1,G2)
    #[test]
    fn bls_pairing_e_2_g1_3_g2_e_6_g1_g2() {
        exec_evm_precompile(PRECOMPILE_BLS12_381_PAIRING,
        &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d2800000000000000000000000000000000122915c824a0857e2ee414a3dccb23ae691ae54329781315a0c75df1c04d6d7a50a030fc866f09d516020ef82324afae0000000000000000000000000000000009380275bbc8e5dcea7dc4dd7e0550ff2ac480905396eda55062650f8d251c96eb480673937cc6d9d6a44aaa56ca66dc000000000000000000000000000000000b21da7955969e61010c7a1abc1a6f0136961d1e3b20b1a7326ac738fef5c721479dfd948b52fdf2455e44813ecfd8920000000000000000000000000000000008f239ba329b3967fe48d718a36cfe5f62a7e42e0bf1c1ed714150a166bfbd6bcf6b3b58b975b9edea56d53f23a0e8490000000000000000000000000000000006e82f6da4520f85c5d27d8f329eccfa05944fd1096b20734c894966d12a9e2a9a9744529d7212d33883113a0cadb9090000000000000000000000000000000017d81038f7d60bee9110d9c0d6d1102fe2d998c957f28e31ec284cc04134df8e47e8f82ff3af2e60a6d9688a4563477c00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed"),
        &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
        161000,
        );
    }
}
