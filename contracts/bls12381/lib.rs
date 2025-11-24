#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;

use alloc::vec::Vec;
use bls12_381::{pairing, G1Affine, G1Projective, G2Affine, G2Projective, Gt, Scalar};
use fluentbase_sdk::{
    system_entrypoint, Bytes, ContextReader, ExitCode, SharedAPI, PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BLS12_381_PAIRING,
};
use revm_precompile::bls12_381::{
    g2_msm::g2_msm, map_fp2_to_g2::map_fp2_to_g2, map_fp_to_g1::map_fp_to_g1,
};

/// BLS12-381 Specific Constants
pub const SCALAR_SIZE: usize = 32;
pub const FP_PAD_BY: usize = 16;
pub const FP_SIZE: usize = 48;
pub const PADDED_FP_SIZE: usize = 64;
pub const FP2_SIZE: usize = 2 * FP_SIZE;
pub const PADDED_FP2_SIZE: usize = 2 * PADDED_FP_SIZE;
pub const PADDED_G1_SIZE: usize = 2 * PADDED_FP_SIZE;
pub const PADDED_G2_SIZE: usize = 2 * PADDED_FP2_SIZE;
pub const G1_UNCOMPRESSED_SIZE: usize = 96;
pub const G1_COMPRESSED_SIZE: usize = 48;
pub const G2_UNCOMPRESSED_SIZE: usize = 192;
pub const G2_COMPRESSED_SIZE: usize = 96;
pub const GT_COMPRESSED_SIZE: usize = 288;

/**
 * This is the BLS12-381 precompile contract.
 *
 * Note: more info on the BLS12-381 curve can be found here: https://eips.ethereum.org/EIPS/eip-2537
 *
 * It implements the following functions:
 * - G1_ADD: patches + syscalls
 * - G1_MSM: patches + syscalls
 * - G2_ADD: patches
 * - G2_MSM: revm_precompile::g2_msm::g2_msm
 * - PAIRING: patches
 * - MAP_G1: revm_precompile::bls12_381::map_fp_to_g1
 * - MAP_G2: revm_precompile::bls12_381::map_fp2_to_g2
 */

///  Gas Constants for the BLS12-381 precompile contract.

const G1_ADD_GAS: u64 = 375u64;
const G2_ADD_GAS: u64 = 600u64;

/// ==== MSM gas constants ====

const MSM_MULTIPLIER: u64 = 1000;
const G1_MSM_GAS: u64 = 12000u64;
pub static DISCOUNT_TABLE_G1_MSM: [u16; 128] = [
    1000, 949, 848, 797, 764, 750, 738, 728, 719, 712, 705, 698, 692, 687, 682, 677, 673, 669, 665,
    661, 658, 654, 651, 648, 645, 642, 640, 637, 635, 632, 630, 627, 625, 623, 621, 619, 617, 615,
    613, 611, 609, 608, 606, 604, 603, 601, 599, 598, 596, 595, 593, 592, 591, 589, 588, 586, 585,
    584, 582, 581, 580, 579, 577, 576, 575, 574, 573, 572, 570, 569, 568, 567, 566, 565, 564, 563,
    562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 551, 550, 549, 548, 547, 547, 546, 545,
    544, 543, 542, 541, 540, 540, 539, 538, 537, 536, 536, 535, 534, 533, 532, 532, 531, 530, 529,
    528, 528, 527, 526, 525, 525, 524, 523, 522, 522, 521, 520, 520, 519,
];

pub static DISCOUNT_TABLE_G2_MSM: [u16; 128] = [
    1000, 1000, 923, 884, 855, 832, 812, 796, 782, 770, 759, 749, 740, 732, 724, 717, 711, 704,
    699, 693, 688, 683, 679, 674, 670, 666, 663, 659, 655, 652, 649, 646, 643, 640, 637, 634, 632,
    629, 627, 624, 622, 620, 618, 615, 613, 611, 609, 607, 606, 604, 602, 600, 598, 597, 595, 593,
    592, 590, 589, 587, 586, 584, 583, 582, 580, 579, 578, 576, 575, 574, 573, 571, 570, 569, 568,
    567, 566, 565, 563, 562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 552, 551, 550, 549,
    548, 547, 546, 545, 545, 544, 543, 542, 541, 541, 540, 539, 538, 537, 537, 536, 535, 535, 534,
    533, 532, 532, 531, 530, 530, 529, 528, 528, 527, 526, 526, 525, 524, 524,
];

/// ==== Pairing gas constants ====

const PAIRING_OFFSET_BASE: u64 = 37700;
const PAIRING_MULTIPLIER_BASE: u64 = 32600;

/// ==== Map gas constants ====

/// ==== Input lengths requirements ====

const G1_ADD_INPUT_LENGTH: usize = 2 * PADDED_G1_SIZE;
const G2_ADD_INPUT_LENGTH: usize = 2 * PADDED_G2_SIZE;

const G1_MSM_INPUT_LENGTH: usize = PADDED_G1_SIZE + 32;

const PAIRING_INPUT_LENGTH: usize = PADDED_G1_SIZE + PADDED_G2_SIZE;

#[inline(always)]
fn msm_required_gas(k: usize, discount_table: &[u16], multiplication_cost: u64) -> u64 {
    if k == 0 {
        return 0;
    }
    let index = core::cmp::min(k - 1, discount_table.len() - 1);
    let discount = discount_table[index] as u64;
    (k as u64 * discount * multiplication_cost) / MSM_MULTIPLIER
}

#[inline(always)]
fn check_gas_and_sync<SDK: SharedAPI>(
    sdk: &SDK,
    gas_used: u64,
    gas_limit: u64,
) -> Result<(), ExitCode> {
    if gas_used > gas_limit {
        return Err(ExitCode::OutOfFuel);
    }
    sdk.sync_evm_gas(gas_used)?;
    Ok(())
}

#[inline(always)]
fn validate_input_length(actual: u32, expected: usize) -> Result<(), ExitCode> {
    if actual != expected as u32 {
        Err(ExitCode::InputOutputOutOfBounds)
    } else {
        Ok(())
    }
}

/// Helper function to convert G1 input from EVM format to runtime format
#[inline(always)]
fn convert_g1_input_to_runtime(
    input: &[u8],
) -> ([u8; G1_UNCOMPRESSED_SIZE], [u8; G1_UNCOMPRESSED_SIZE]) {
    let a = &input[0..PADDED_G1_SIZE];
    let b = &input[PADDED_G1_SIZE..2 * PADDED_G1_SIZE];

    let (x1_be, y1_be) = (
        &a[0..PADDED_FP_SIZE],
        &a[PADDED_FP_SIZE..2 * PADDED_FP_SIZE],
    );
    let (x2_be, y2_be) = (
        &b[0..PADDED_FP_SIZE],
        &b[PADDED_FP_SIZE..2 * PADDED_FP_SIZE],
    );

    let mut p = [0u8; G1_UNCOMPRESSED_SIZE];
    let mut q = [0u8; G1_UNCOMPRESSED_SIZE];

    // Helper function to copy 48-byte field from padded input
    let copy_field = |dst: &mut [u8], src: &[u8]| {
        dst.copy_from_slice(&src[FP_PAD_BY..PADDED_FP_SIZE]);
    };

    // p (x, y)
    copy_field(&mut p[0..FP_SIZE], x1_be);
    copy_field(&mut p[FP_SIZE..G1_UNCOMPRESSED_SIZE], y1_be);
    // q (x, y)
    copy_field(&mut q[0..FP_SIZE], x2_be);
    copy_field(&mut q[FP_SIZE..G1_UNCOMPRESSED_SIZE], y2_be);

    (p, q)
}

/// Helper function to convert G2 input from EVM format to rwasm-patches format
#[inline(always)]
fn convert_g2_input_to_rwasm_patches(
    input: &[u8],
) -> ([u8; G2_UNCOMPRESSED_SIZE], [u8; G2_UNCOMPRESSED_SIZE]) {
    let a = &input[0..PADDED_G2_SIZE];
    let b = &input[PADDED_G2_SIZE..(2 * PADDED_G2_SIZE)];
    let (a_x0, a_x1, a_y0, a_y1) = (
        &a[0..PADDED_FP_SIZE],
        &a[PADDED_FP_SIZE..(2 * PADDED_FP_SIZE)],
        &a[(2 * PADDED_FP_SIZE)..(3 * PADDED_FP_SIZE)],
        &a[(3 * PADDED_FP_SIZE)..(4 * PADDED_FP_SIZE)],
    );
    let (b_x0, b_x1, b_y0, b_y1) = (
        &b[0..PADDED_FP_SIZE],
        &b[PADDED_FP_SIZE..(2 * PADDED_FP_SIZE)],
        &b[(2 * PADDED_FP_SIZE)..(3 * PADDED_FP_SIZE)],
        &b[(3 * PADDED_FP_SIZE)..(4 * PADDED_FP_SIZE)],
    );

    let mut p = [0u8; G2_UNCOMPRESSED_SIZE];
    let mut q = [0u8; G2_UNCOMPRESSED_SIZE];

    // Helper function to convert G2 field elements to rwasm-patches format
    // Format: x.c1 (48 bytes) || x.c0 (48 bytes) || y.c1 (48 bytes) || y.c0 (48 bytes)
    let convert_g2_fields_rwasm = |dst: &mut [u8], x0: &[u8], x1: &[u8], y0: &[u8], y1: &[u8]| {
        let copy_fp_field = |dst: &mut [u8], src: &[u8]| {
            // Extract the 48-byte field element (skip the 16-byte padding)
            let field_bytes = &src[FP_PAD_BY..PADDED_FP_SIZE];
            dst.copy_from_slice(field_bytes);
        };

        // x.c1 (high part) - bytes 0-47
        copy_fp_field(&mut dst[0..48], x1);
        // x.c0 (low part) - bytes 48-95
        copy_fp_field(&mut dst[48..96], x0);
        // y.c1 (high part) - bytes 96-143
        copy_fp_field(&mut dst[96..144], y1);
        // y.c0 (low part) - bytes 144-191
        copy_fp_field(&mut dst[144..192], y0);
    };

    // Convert a and b G2 points to rwasm-patches format
    convert_g2_fields_rwasm(&mut p, a_x0, a_x1, a_y0, a_y1);
    convert_g2_fields_rwasm(&mut q, b_x0, b_x1, b_y0, b_y1);

    (p, q)
}

/// Helper function to convert G1 output from runtime format to EVM format
#[inline(always)]
fn convert_g1_output_to_evm(p: &[u8; G1_UNCOMPRESSED_SIZE]) -> [u8; PADDED_G1_SIZE] {
    let mut out = [0u8; PADDED_G1_SIZE];
    // x: 48 bytes -> place at [16..64] (no byte order conversion needed)
    out[FP_PAD_BY..PADDED_FP_SIZE].copy_from_slice(&p[0..FP_SIZE]);

    // y: 48 bytes -> place at [80..128] (no byte order conversion needed)
    out[80..PADDED_G1_SIZE].copy_from_slice(&p[FP_SIZE..G1_UNCOMPRESSED_SIZE]);
    out
}

/// Helper function to convert G2 output from rwasm-patches format to EVM format
#[inline(always)]
fn convert_g2_output_to_evm_rwasm(p: &[u8; G2_UNCOMPRESSED_SIZE]) -> [u8; PADDED_G2_SIZE] {
    let mut out = [0u8; PADDED_G2_SIZE];

    // rwasm-patches format: x.c1 (0-47) || x.c0 (48-95) || y.c1 (96-143) || y.c0 (144-191)
    // EVM format: x.c0 || x.c1 || y.c0 || y.c1, each 64-byte BE padded

    // x.c0: bytes 48-95 -> first padded slot
    out[FP_PAD_BY..PADDED_FP_SIZE].copy_from_slice(&p[48..96]);
    // x.c1: bytes 0-47 -> second padded slot
    out[(PADDED_FP_SIZE + FP_PAD_BY)..(2 * PADDED_FP_SIZE)].copy_from_slice(&p[0..48]);
    // y.c0: bytes 144-191 -> third padded slot
    out[(2 * PADDED_FP_SIZE + FP_PAD_BY)..(3 * PADDED_FP_SIZE)].copy_from_slice(&p[144..192]);
    // y.c1: bytes 96-143 -> fourth padded slot
    out[(3 * PADDED_FP_SIZE + FP_PAD_BY)..(4 * PADDED_FP_SIZE)].copy_from_slice(&p[96..144]);

    out
}

/// Helper function to convert G1 point from EVM format to rwasm-patches format for pairing
#[inline(always)]
fn convert_g1_input_to_rwasm_patches_pairing(input: &[u8]) -> [u8; G1_UNCOMPRESSED_SIZE] {
    let mut out = [0u8; G1_UNCOMPRESSED_SIZE];

    // EVM format: x (64-byte BE padded) || y (64-byte BE padded)
    // rwasm-patches format: x (48 bytes) || y (48 bytes)
    out[0..48].copy_from_slice(&input[FP_PAD_BY..PADDED_FP_SIZE]);
    out[48..96].copy_from_slice(&input[PADDED_FP_SIZE + FP_PAD_BY..2 * PADDED_FP_SIZE]);
    out
}

/// Helper function to convert G2 point from EVM format to rwasm-patches format for pairing
#[inline(always)]
fn convert_g2_input_to_rwasm_patches_pairing(input: &[u8]) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let mut out = [0u8; G2_UNCOMPRESSED_SIZE];

    // EVM format: x0 (64-byte BE padded) || x1 (64-byte BE padded) || y0 (64-byte BE padded) || y1 (64-byte BE padded)
    // rwasm-patches format: x.c1 (48 bytes) || x.c0 (48 bytes) || y.c1 (48 bytes) || y.c0 (48 bytes)

    // x.c1: bytes 0-47 (from x1)
    out[0..48].copy_from_slice(&input[PADDED_FP_SIZE + FP_PAD_BY..2 * PADDED_FP_SIZE]);
    // x.c0: bytes 48-95 (from x0)
    out[48..96].copy_from_slice(&input[FP_PAD_BY..PADDED_FP_SIZE]);
    // y.c1: bytes 96-143 (from y1)
    out[96..144].copy_from_slice(&input[3 * PADDED_FP_SIZE + FP_PAD_BY..4 * PADDED_FP_SIZE]);
    // y.c0: bytes 144-191 (from y0)
    out[144..192].copy_from_slice(&input[2 * PADDED_FP_SIZE + FP_PAD_BY..3 * PADDED_FP_SIZE]);

    out
}

/// Helper function to convert G1 point from EVM format to rwasm-patches format for MSM
#[inline(always)]
fn convert_g1_input_to_rwasm_patches_msm(input: &[u8]) -> [u8; G1_UNCOMPRESSED_SIZE] {
    let mut out = [0u8; G1_UNCOMPRESSED_SIZE];

    // Use the same logic as convert_g1_input_to_runtime for consistency
    let (x_be, y_be) = (
        &input[0..PADDED_FP_SIZE],
        &input[PADDED_FP_SIZE..2 * PADDED_FP_SIZE],
    );

    // Helper function to copy 48-byte field from padded input
    let copy_field = |dst: &mut [u8], src: &[u8]| {
        dst.copy_from_slice(&src[FP_PAD_BY..PADDED_FP_SIZE]);
    };

    // Copy x and y coordinates
    copy_field(&mut out[0..FP_SIZE], x_be);
    copy_field(&mut out[FP_SIZE..G1_UNCOMPRESSED_SIZE], y_be);

    out
}

/// Helper function to convert scalar from BE format to rwasm-patches Scalar
#[inline(always)]
fn convert_scalar_be_to_rwasm_patches(scalar_be: &[u8; SCALAR_SIZE]) -> Scalar {
    // Convert from BE bytes to LE bytes, then to Scalar
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(scalar_be);
    bytes.reverse(); // Convert BE to LE
    Scalar::from_bytes(&bytes).unwrap_or(Scalar::zero())
}

/// Helper function for common validation and gas checking pattern
#[inline(always)]
fn validate_and_consume_gas<SDK: SharedAPI>(
    sdk: &SDK,
    input_length: u32,
    expected_length: usize,
    gas_cost: u64,
    gas_limit: u64,
) -> Result<(), ExitCode> {
    validate_input_length(input_length, expected_length)?;
    check_gas_and_sync(sdk, gas_cost, gas_limit)?;
    Ok(())
}

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, ExitCode> {
    // read full input data
    let bytecode_address = sdk.context().contract_bytecode_address();
    let gas_limit = sdk.context().contract_gas_limit();
    let input_length = sdk.input_size();
    let input = sdk.input();
    // dispatch to SDK-backed implementation
    match bytecode_address {
        PRECOMPILE_BLS12_381_G1_ADD => {
            // Expect two G1 points (x1||y1||x2||y2), each coord 64 bytes BE padded
            validate_and_consume_gas(
                sdk,
                input_length,
                G1_ADD_INPUT_LENGTH,
                G1_ADD_GAS,
                gas_limit,
            )?;
            // Convert input from EVM format to runtime format
            let (p, q) = convert_g1_input_to_runtime(&input);
            // Use rWASM-patched bls12_381 directly for optimized execution
            let p_aff = G1Affine::from_uncompressed(&p).unwrap_or(G1Affine::identity());
            let q_aff = G1Affine::from_uncompressed(&q).unwrap_or(G1Affine::identity());

            let result = p_aff.add_affine(&q_aff);
            let result_bytes = result.to_uncompressed();

            // Convert output from runtime format to EVM format
            let out = convert_g1_output_to_evm(&result_bytes);
            Ok(out.into())
        }
        PRECOMPILE_BLS12_381_G2_ADD => {
            // EIP-2537: input must be 512 bytes (two G2 elements, each 256 bytes padded)
            validate_and_consume_gas(
                sdk,
                input_length,
                G2_ADD_INPUT_LENGTH,
                G2_ADD_GAS,
                gas_limit,
            )?;

            // Convert input from EVM format to runtime format
            let (p, q) = convert_g2_input_to_rwasm_patches(&input);

            let p_aff = G2Affine::from_uncompressed(&p).unwrap_or(G2Affine::identity());
            let q_aff = G2Affine::from_uncompressed(&q).unwrap_or(G2Affine::identity());

            let result = G2Projective::from(p_aff) + G2Projective::from(q_aff);
            let result_aff = G2Affine::from(result);
            let result_bytes = result_aff.to_uncompressed();

            // Encode output: 256 bytes (x0||x1||y0||y1), each limb is 64-byte BE padded (16 zeros + 48 value)
            let out = convert_g2_output_to_evm_rwasm(&result_bytes);
            Ok(out.into())
        }
        PRECOMPILE_BLS12_381_G1_MSM => {
            // Expect pairs of 160 bytes: 128-byte padded G1 point (x||y) + 32-byte scalar (BE)
            // Convert to rwasm-patches format: 96-byte uncompressed G1 + Scalar
            let input_length_requirement = G1_MSM_INPUT_LENGTH;
            if input.len() % input_length_requirement != 0 || input.is_empty() {
                return Err(ExitCode::InputOutputOutOfBounds);
            }
            let pairs_len = input.len() / input_length_requirement;

            // We check for the gas in the very beginning to reduce execution time
            let gas_used = msm_required_gas(pairs_len, &DISCOUNT_TABLE_G1_MSM, G1_MSM_GAS);
            check_gas_and_sync(sdk, gas_used, gas_limit)?;

            // Collect G1 points and scalars for MSM
            let mut points: Vec<G1Projective> = Vec::with_capacity(pairs_len);
            let mut scalars: Vec<Scalar> = Vec::with_capacity(pairs_len);

            for i in 0..pairs_len {
                let start = i * input_length_requirement;
                let g1_in = &input[start..start + PADDED_G1_SIZE];
                let s_be = &input[start + PADDED_G1_SIZE..start + input_length_requirement];

                // Convert G1 point from EVM format to rwasm-patches format
                let g1_bytes = convert_g1_input_to_rwasm_patches_msm(g1_in);
                let g1_aff = G1Affine::from_uncompressed(&g1_bytes).unwrap_or(G1Affine::identity());
                let g1_proj = G1Projective::from(g1_aff);

                // Convert scalar from BE format to rwasm-patches Scalar
                let mut s_be_array = [0u8; SCALAR_SIZE];
                s_be_array.copy_from_slice(s_be);
                let scalar = convert_scalar_be_to_rwasm_patches(&s_be_array);

                points.push(g1_proj);
                scalars.push(scalar);
            }

            // Perform MSM using rwasm-patches
            let result = G1Projective::msm_variable_base(&points, &scalars);
            let result_aff = G1Affine::from(result);
            let result_bytes = result_aff.to_uncompressed();

            // Check if the result is identity
            let out = if result_aff.is_identity().unwrap_u8() == 1 {
                [0u8; PADDED_G1_SIZE]
            } else {
                convert_g1_output_to_evm(&result_bytes)
            };
            Ok(out.into())
        }
        PRECOMPILE_BLS12_381_G2_MSM => {
            let output = g2_msm(&input, gas_limit).map_err(|_| ExitCode::InputOutputOutOfBounds)?;
            // Consume the gas that was used by to precompile
            sdk.sync_evm_gas(output.gas_used)?;
            Ok(output.bytes)
        }
        PRECOMPILE_BLS12_381_PAIRING => {
            if input.is_empty() || input.len() % PAIRING_INPUT_LENGTH != 0 {
                return Err(ExitCode::InputOutputOutOfBounds);
            }
            let pairs_len = input.len() / PAIRING_INPUT_LENGTH;
            // Gas: PAIRING_MULTIPLIER_BASE * pairs + PAIRING_OFFSET_BASE
            let required_gas = PAIRING_MULTIPLIER_BASE
                .saturating_mul(pairs_len as u64)
                .saturating_add(PAIRING_OFFSET_BASE);
            if required_gas > gas_limit {
                return Err(ExitCode::OutOfFuel);
            }
            sdk.sync_evm_gas(required_gas)?;

            // Process each pair and compute the product of all pairings
            let mut result = Gt::identity();

            for i in 0..pairs_len {
                let start = i * PAIRING_INPUT_LENGTH;

                // Convert G1 point from EVM format to rwasm-patches format
                let g1_bytes = convert_g1_input_to_rwasm_patches_pairing(
                    &input[start..start + PADDED_G1_SIZE],
                );
                let g1_aff = G1Affine::from_uncompressed(&g1_bytes).unwrap_or(G1Affine::identity());

                // Convert G2 point from EVM format to rwasm-patches format
                let g2_bytes = convert_g2_input_to_rwasm_patches_pairing(
                    &input[start + PADDED_G1_SIZE..start + PAIRING_INPUT_LENGTH],
                );
                let g2_aff = G2Affine::from_uncompressed(&g2_bytes).unwrap_or(G2Affine::identity());

                // Compute pairing for this pair
                let pair_result = pairing(&g1_aff, &g2_aff);

                // Add to the running result (multiplication in GT group)
                result = result + pair_result;
            }

            // Return 1 if result is identity (all pairings multiply to 1), 0 otherwise
            let is_one = result == Gt::identity();
            let mut out_be = [0u8; 32];
            if is_one {
                out_be[31] = 1;
            }
            Ok(out_be.into())
        }
        PRECOMPILE_BLS12_381_MAP_G1 => {
            // Use revm_precompile directly for MAP_G1
            let output =
                map_fp_to_g1(&input, gas_limit).map_err(|_| ExitCode::InputOutputOutOfBounds)?;
            // Consume the gas that was used by to precompile
            sdk.sync_evm_gas(output.gas_used)?;
            Ok(output.bytes)
        }
        PRECOMPILE_BLS12_381_MAP_G2 => {
            // Use revm_precompile directly for MAP_G2
            let output =
                map_fp2_to_g2(&input, gas_limit).map_err(|_| ExitCode::InputOutputOutOfBounds)?;
            // Consume the gas that was used by to precompile
            sdk.sync_evm_gas(output.gas_used)?;
            Ok(output.bytes)
        }
        _ => unreachable!("bls12381: unsupported contract address"),
    }
}

system_entrypoint!(main_entry);

/**
 * The following are the tests for the BLS12-381 precompile contract.
 *
 * Note: The tests cases are taken from the: https://eips.ethereum.org/assets/eip-2537/test-vectors
 */

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_testing::HostTestingContext;

    fn exec_evm_precompile(address: Address, inputs: &[u8], expected: &[u8], expected_gas: u64) {
        let gas_limit = 120_000;
        let mut sdk = HostTestingContext::default()
            .with_input(Bytes::copy_from_slice(inputs))
            .with_contract_context(ContractContextV1 {
                address,
                bytecode_address: address,
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        let output = main_entry(&mut sdk).unwrap();
        assert_eq!(output.as_ref(), expected);
        let gas_remaining = sdk.fuel() / FUEL_DENOM_RATE;
        assert_eq!(gas_limit - gas_remaining, expected_gas);
    }

    // ==================================== G1 ADD ====================================
    mod g1_add {
        use super::*;
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
    }
    // ==================================== G2 ADD ====================================
    mod g2_add {
        use super::*;
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
    }
    // ==================================== G1 MSM ====================================
    mod g1_msm {
        use super::*;
        #[test]
        fn bls_g1msm_g1_add_g1_2_g1() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28"),
            12000,
        );
        }
        #[test]
        fn bls_g1msm_p1_add_p1_2_p1() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("0000000000000000000000000000000015222cddbabdd764c4bee0b3720322a65ff4712c86fc4b1588d0c209210a0884fa9468e855d261c483091b2bf7de6a630000000000000000000000000000000009f9edb99bc3b75d7489735c98b16ab78b9386c5f7a1f76c7e96ac6eb5bbde30dbca31a74ec6e0f0b12229eecea33c39"),
            12000,
        );
        }
        #[test]
        fn bls_g1msm_1_mul_g1_g1() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1"),
            12000,
        );
        }
        #[test]
        fn bls_g1msm_1_mul_p1_p1() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000001"),
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21"),
            12000,
        );
        }
        #[test]
        fn bls_g1msm_0_mul_g1_inf() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            12000,
        );
        }
        #[test]
        fn bls_g1msm_0_mul_p1_inf() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G1_MSM,
            &hex!("00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000000"),
            &hex!("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
            12000,
        );
        }
    }
    // ==================================== G2 MSM ====================================
    mod g2_msm {
        use super::*;
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
        #[test]
        fn bls_g2mul_2_g2_add_2_p2() {
            exec_evm_precompile(
            PRECOMPILE_BLS12_381_G2_MSM,
            &hex!("00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d8784510000000000000000000000000000000000000000000000000000000000000002"),
            &hex!("00000000000000000000000000000000009cc9ed6635623ba19b340cbc1b0eb05c3a58770623986bb7e041645175b0a38d663d929afb9a949f7524656043bccc000000000000000000000000000000000c0fb19d3f083fd5641d22a861a11979da258003f888c59c33005cb4a2df4df9e5a2868832063ac289dfa3e997f21f8a00000000000000000000000000000000168bf7d87cef37cf1707849e0a6708cb856846f5392d205ae7418dd94d94ef6c8aa5b424af2e99d957567654b9dae1d90000000000000000000000000000000017e0fa3c3b2665d52c26c7d4cea9f35443f4f9007840384163d3aa3c7d4d18b21b65ff4380cf3f3b48e94b5eecb221dd"),
            45000,
        );
        }
    }
    // ==================================== Pairing ====================================
    mod pairing {
        use super::*;
        #[test]
        fn bls_pairing_e_0_0() {
            exec_evm_precompile(
                PRECOMPILE_BLS12_381_PAIRING,
                &hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                70300,
            );
        }
        // bls_pairing_e(0,0)=e(0,0)
        #[test]
        fn bls_pairing_e_0_0_e_0_0() {
            exec_evm_precompile(
                PRECOMPILE_BLS12_381_PAIRING,
                &hex!("000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                &hex!("0000000000000000000000000000000000000000000000000000000000000001"),
                102900,
            );
        }
    }
    // ==================================== Map_Fp1_to_G1 ====================================
    mod map_to_g1 {
        use super::*;
        #[test]
        fn bls_map_g1() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G1,
        &hex!("00000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f03"),
        &hex!("00000000000000000000000000000000184bb665c37ff561a89ec2122dd343f20e0f4cbcaec84e3c3052ea81d1834e192c426074b02ed3dca4e7676ce4ce48ba0000000000000000000000000000000004407b8d35af4dacc809927071fc0405218f1401a6d15af775810e4e460064bcc9468beeba82fdc751be70476c888bf3"),
        5500,
        );
        }
        #[test]
        fn bls_map_g1_616263() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G1,
        &hex!("00000000000000000000000000000000147e1ed29f06e4c5079b9d14fc89d2820d32419b990c1c7bb7dbea2a36a045124b31ffbde7c99329c05c559af1c6cc82"),
        &hex!("00000000000000000000000000000000009769f3ab59bfd551d53a5f846b9984c59b97d6842b20a2c565baa167945e3d026a3755b6345df8ec7e6acb6868ae6d000000000000000000000000000000001532c00cf61aa3d0ce3e5aa20c3b531a2abd2c770a790a2613818303c6b830ffc0ecf6c357af3317b9575c567f11cd2c"),
        5500,
        );
        }
        #[test]
        fn bls_g1map_6162636465663031() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G1,
        &hex!("0000000000000000000000000000000004090815ad598a06897dd89bcda860f25837d54e897298ce31e6947378134d3761dc59a572154963e8c954919ecfa82d"),
        &hex!("000000000000000000000000000000001974dbb8e6b5d20b84df7e625e2fbfecb2cdb5f77d5eae5fb2955e5ce7313cae8364bc2fff520a6c25619739c6bdcb6a0000000000000000000000000000000015f9897e11c6441eaa676de141c8d83c37aab8667173cbe1dfd6de74d11861b961dccebcd9d289ac633455dfcc7013a3"),
        5500,
        );
        }
    }
    // ==================================== Map_Fp2_to_G2 ====================================
    mod map_to_g2 {
        use super::*;
        #[test]
        fn bls_map_g2() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G2,
         &hex!("0000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b040000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b78c"),
         &hex!("0000000000000000000000000000000000e7f4568a82b4b7dc1f14c6aaa055edf51502319c723c4dc2688c7fe5944c213f510328082396515734b6612c4e7bb700000000000000000000000000000000126b855e9e69b1f691f816e48ac6977664d24d99f8724868a184186469ddfd4617367e94527d4b74fc86413483afb35b000000000000000000000000000000000caead0fd7b6176c01436833c79d305c78be307da5f6af6c133c47311def6ff1e0babf57a0fb5539fce7ee12407b0a42000000000000000000000000000000001498aadcf7ae2b345243e281ae076df6de84455d766ab6fcdaad71fab60abb2e8b980a440043cd305db09d283c895e3d"),
         23800,
         );
        }
        #[test]
        fn bls_g2map_616263() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G2,
         &hex!("00000000000000000000000000000000138879a9559e24cecee8697b8b4ad32cced053138ab913b99872772dc753a2967ed50aabc907937aefb2439ba06cc50c000000000000000000000000000000000a1ae7999ea9bab1dcc9ef8887a6cb6e8f1e22566015428d220b7eec90ffa70ad1f624018a9ad11e78d588bd3617f9f2"),
         &hex!("00000000000000000000000000000000108ed59fd9fae381abfd1d6bce2fd2fa220990f0f837fa30e0f27914ed6e1454db0d1ee957b219f61da6ff8be0d6441f000000000000000000000000000000000296238ea82c6d4adb3c838ee3cb2346049c90b96d602d7bb1b469b905c9228be25c627bffee872def773d5b2a2eb57d00000000000000000000000000000000033f90f6057aadacae7963b0a0b379dd46750c1c94a6357c99b65f63b79e321ff50fe3053330911c56b6ceea08fee65600000000000000000000000000000000153606c417e59fb331b7ae6bce4fbf7c5190c33ce9402b5ebe2b70e44fca614f3f1382a3625ed5493843d0b0a652fc3f"),
         23800,
         );
        }
        #[test]
        fn bls_g2map_6162636465663031() {
            exec_evm_precompile(PRECOMPILE_BLS12_381_MAP_G2,
         &hex!("0000000000000000000000000000000018c16fe362b7dbdfa102e42bdfd3e2f4e6191d479437a59db4eb716986bf08ee1f42634db66bde97d6c16bbfd342b3b8000000000000000000000000000000000e37812ce1b146d998d5f92bdd5ada2a31bfd63dfe18311aa91637b5f279dd045763166aa1615e46a50d8d8f475f184e"),
         &hex!("00000000000000000000000000000000038af300ef34c7759a6caaa4e69363cafeed218a1f207e93b2c70d91a1263d375d6730bd6b6509dcac3ba5b567e85bf3000000000000000000000000000000000da75be60fb6aa0e9e3143e40c42796edf15685cafe0279afd2a67c3dff1c82341f17effd402e4f1af240ea90f4b659b0000000000000000000000000000000019b148cbdf163cf0894f29660d2e7bfb2b68e37d54cc83fd4e6e62c020eaa48709302ef8e746736c0e19342cc1ce3df4000000000000000000000000000000000492f4fed741b073e5a82580f7c663f9b79e036b70ab3e51162359cec4e77c78086fe879b65ca7a47d34374c8315ac5e"),
         23800,
         );
        }
    }
}
