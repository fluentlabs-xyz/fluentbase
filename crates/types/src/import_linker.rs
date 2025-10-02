use crate::{emit_fuel_procedure, SysFuncIdx};
use alloc::sync::Arc;
use rwasm::{ImportLinker, ImportName, ValType};

#[rustfmt::skip]
pub fn import_linker_v1_preview() -> Arc<ImportLinker> {
    let mut import_linker = ImportLinker::default();
    macro_rules! import_function {
        ($func_name:literal, $sys_func_idx:ident, $params:expr, $results:expr $(,)?) => {
            import_linker.insert_function(
                ImportName::new("fluentbase_v1preview", $func_name),
                SysFuncIdx::$sys_func_idx as u32,
                emit_fuel_procedure(SysFuncIdx::$sys_func_idx),
                $params,
                $results,
            );
        };
    }

    // input/output & state control (0x00)
    import_function!("_exit", EXIT, &[ValType::I32; 1], &[]);
    import_function!("_state", STATE, &[], &[ValType::I32; 1]);
    import_function!("_read", READ_INPUT, &[ValType::I32; 3], &[]);
    import_function!("_input_size", INPUT_SIZE, &[], &[ValType::I32; 1]);
    import_function!("_write", WRITE_OUTPUT, &[ValType::I32; 2], &[]);
    import_function!("_output_size", OUTPUT_SIZE, &[], &[ValType::I32; 1]);
    import_function!("_read_output", READ_OUTPUT, &[ValType::I32; 3], &[]);
    import_function!("_exec", EXEC, &[ValType::I32; 5], &[ValType::I32; 1]);
    import_function!("_resume", RESUME, &[ValType::I32; 5], &[ValType::I32; 1]);
    import_function!("_forward_output", FORWARD_OUTPUT, &[ValType::I32; 2], &[]);
    import_function!("_charge_fuel_manually", CHARGE_FUEL_MANUALLY, &[ValType::I64; 2], &[ValType::I64; 1]);
    import_function!("_fuel", FUEL, &[], &[ValType::I64; 1]);
    import_function!("_debug_log", DEBUG_LOG, &[ValType::I32; 2], &[]);
    import_function!("_charge_fuel", CHARGE_FUEL, &[ValType::I64; 1], &[]);

    // hashing functions (0x01)
    import_function!("_keccak256", KECCAK256, &[ValType::I32; 3], &[]);
    import_function!("_keccak256_permute", KECCAK256_PERMUTE, &[ValType::I32; 1], &[]);
    import_function!("_poseidon", POSEIDON, &[ValType::I32; 5], &[ValType::I32; 1]);
    // TODO: Delete "_poseidon_hash"
    import_function!("_sha256_extend", SHA256_EXTEND, &[ValType::I32; 1], &[]);
    import_function!("_sha256_compress", SHA256_COMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_sha256", SHA256, &[ValType::I32; 3], &[]);
    import_function!("_blake3", BLAKE3, &[ValType::I32; 3], &[]);

    // ed25519 (0x02)
    import_function!("_ed25519_decompress", ED25519_DECOMPRESS, &[ValType::I32; 1], &[ValType::I32; 1]);
    import_function!("_ed25519_add", ED25519_ADD, &[ValType::I32; 2], &[ValType::I32; 1]);

        // fp1/fp2 tower field (0x03)
    import_function!("_tower_fp1_bn254_add", TOWER_FP1_BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bn254_sub", TOWER_FP1_BN254_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bn254_mul", TOWER_FP1_BN254_MUL, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_add", TOWER_FP1_BLS12381_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_sub", TOWER_FP1_BLS12381_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_mul", TOWER_FP1_BLS12381_MUL, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bn254_add", TOWER_FP2_BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bn254_sub", TOWER_FP2_BN254_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bn254_mul", TOWER_FP2_BN254_MUL, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bls12381_add", TOWER_FP2_BLS12381_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bls12381_sub", TOWER_FP2_BLS12381_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bls12381_mul", TOWER_FP2_BLS12381_MUL, &[ValType::I32; 2], &[]);

    // secp256k1 (0x04)
    import_function!("_secp256k1_add", SECP256K1_ADD, &[ValType::I32; 2], &[]);
    import_function!("_secp256k1_decompress", SECP256K1_DECOMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_secp256k1_double", SECP256K1_DOUBLE, &[ValType::I32; 1], &[]);

    // bls12381 (0x06)
    import_function!("_bls12381_g1_add", BLS12381_G1_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bls12381_g1_msm", BLS12381_G1_MSM, &[ValType::I32; 3], &[]);
    import_function!("_bls12381_g2_add", BLS12381_G2_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bls12381_g2_msm", BLS12381_G2_MSM, &[ValType::I32; 3], &[]);
    import_function!("_bls12381_pairing", BLS12381_PAIRING, &[ValType::I32; 3], &[]);
    import_function!("_bls12381_map_g1", BLS12381_MAP_G1, &[ValType::I32; 2],&[]);
    import_function!("_bls12381_map_g2", BLS12381_MAP_G2, &[ValType::I32; 2], &[]);

    // bn254 (0x07)
    import_function!("_bn254_add", BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bn254_double", BN254_DOUBLE, &[ValType::I32; 1], &[]);
    import_function!("_bn254_mul", BN254_MUL, &[ValType::I32; 2], &[]);
    import_function!(
        "_bn254_multi_pairing",
        BN254_MULTI_PAIRING,
        &[ValType::I32; 3],
        &[]
    );
    import_function!(
        "_bn254_g1_compress",
        BN254_G1_COMPRESS,
        &[ValType::I32; 2],
        &[ValType::I32; 1]
    );
    import_function!(
        "_bn254_g1_decompress",
        BN254_G1_DECOMPRESS,
        &[ValType::I32; 2],
        &[ValType::I32; 1]
    );
    import_function!(
        "_bn254_g2_compress",
        BN254_G2_COMPRESS,
        &[ValType::I32; 2],
        &[ValType::I32; 1]
    );
    import_function!(
        "_bn254_g2_decompress",
        BN254_G2_DECOMPRESS,
        &[ValType::I32; 2],
        &[ValType::I32; 1]
    );

    Arc::new(import_linker)
}
