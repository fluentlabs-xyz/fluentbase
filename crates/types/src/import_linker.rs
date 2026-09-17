use crate::{calculate_syscall_fuel, SysFuncIdx};
use alloc::sync::Arc;
use rwasm::{ImportLinker, ImportName, ValType};

/// Builds the `fluentbase_v1preview` import linker: the syscall ABI that every system-runtime
/// hint and every rWasm contract links against.
///
/// # Compatibility rules
///
/// This table is consensus data, not an internal API. Once a module importing an entry has
/// executed on a live network, that entry belongs to the network's history: a node that
/// re-executes those blocks or serves RPC at those heights loads the hint from state and links
/// it against the linker compiled into the node. `SystemRuntime::new` runs the rWasm front end
/// over every hint before instantiation, so an import that no longer resolves fails the whole
/// frame with `MalformedBuiltinParams` and the full fuel limit consumed, for every system
/// runtime in the affected range.
///
/// - Append only. Never remove an entry or change its name, parameter or result types, index,
///   or observable behaviour (fuel included) once it has shipped. Commenting out the
///   `SysFuncIdx` value reserves the number but does not keep the name resolvable.
/// - Breaking changes go into a new namespace (a new module name next to
///   `fluentbase_v1preview`); old hints keep linking against this one.
/// - A semantic change to an existing syscall changes the result of every historical hint that
///   calls it and needs a fork height or a code-hash gate, like the other chain rules.
/// - Removing a handler declares the history that used it unreproducible. Do it only together
///   with a documented height below which that network is served from snapshots.
/// - Admission (`validate_system_runtime`) checks new hints against this table. Nothing checks
///   this table against hints already in state, so verify a change by loading every hint
///   generation ever present at a system address on testnet and mainnet. Discover them from
///   state (`eth_getCode` change points), not only from runtime-upgrade events: the early
///   testnet generations predate the upgrade contract.
///
/// # Precedent
///
/// `_charge_fuel_manually`, `_secp256k1_recover`, `_bn254_decompress` and the `_bls12_381_*`
/// family were removed in v0.5.x (#188, #201, #217). Fluent Testnet (chain 20994) hints
/// installed before block 20,721,329 (2026-03-05; ecrecover until 20,721,448) still import
/// `_charge_fuel_manually`, `_secp256k1_recover` and `_bls12_381_*`, so on v1.5.0 nodes every
/// `eth_call` or trace at those heights halts with `MalformedBuiltinParams` and archive nodes
/// log `revm: failed to decode structured runtime execution outcome`. Mainnet's genesis hints
/// were built after the removals and are unaffected.
#[rustfmt::skip]
pub fn import_linker_v1_preview() -> Arc<ImportLinker> {
    let mut import_linker = ImportLinker::default();
    macro_rules! import_function {
        ($func_name:literal, $sys_func_idx:ident, $params:expr, $results:expr $(,)?) => {
            import_linker.insert_function(
                ImportName::new("fluentbase_v1preview", $func_name),
                SysFuncIdx::$sys_func_idx as u32,
                calculate_syscall_fuel(SysFuncIdx::$sys_func_idx),
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
    import_function!("_fuel", FUEL, &[], &[ValType::I64; 1]);
    import_function!("_debug_log", DEBUG_LOG, &[ValType::I32; 2], &[]);
    import_function!("_charge_fuel", CHARGE_FUEL, &[ValType::I64; 1], &[]);
    import_function!("_enter_unconstrained", ENTER_UNCONSTRAINED, &[], &[]);
    import_function!("_exit_unconstrained", EXIT_UNCONSTRAINED, &[], &[]);
    // TODO(dmitry123): This syscall is disabled since it can cause panic, we should refine it
    //  by introducing new system contracts where the same functionality is achieved.
    // import_function!("_write_fd", WRITE_FD, &[ValType::I32; 3], &[]);

    // hashing functions (0x01)
    import_function!("_keccak256", KECCAK256, &[ValType::I32; 3], &[]);
    import_function!("_keccak256_permute", KECCAK256_PERMUTE, &[ValType::I32; 1], &[]);
    // import_function!("_poseidon", POSEIDON, &[ValType::I32; 5], &[ValType::I32; 1]);
    import_function!("_sha256_extend", SHA256_EXTEND, &[ValType::I32; 1], &[]);
    import_function!("_sha256_compress", SHA256_COMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_sha256", SHA256, &[ValType::I32; 3], &[]);
    import_function!("_blake3", BLAKE3, &[ValType::I32; 3], &[]);

    // ed25519 (0x02)
    import_function!("_ed25519_decompress", ED25519_DECOMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_ed25519_add", ED25519_ADD, &[ValType::I32; 2], &[]);

    // fp1/fp2 tower field (0x03)
    import_function!("_tower_fp1_bn254_add", TOWER_FP1_BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bn254_sub", TOWER_FP1_BN254_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bn254_mul", TOWER_FP1_BN254_MUL, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_add", TOWER_FP1_BLS12381_ADD, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_sub", TOWER_FP1_BLS12381_SUB, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp1_bls12381_mul", TOWER_FP1_BLS12381_MUL, &[ValType::I32; 2], &[]);
    import_function!("_tower_fp2_bn254_add", TOWER_FP2_BN254_ADD, &[ValType::I32; 4], &[]);
    import_function!("_tower_fp2_bn254_sub", TOWER_FP2_BN254_SUB, &[ValType::I32; 4], &[]);
    import_function!("_tower_fp2_bn254_mul", TOWER_FP2_BN254_MUL, &[ValType::I32; 4], &[]);
    import_function!("_tower_fp2_bls12381_add", TOWER_FP2_BLS12381_ADD, &[ValType::I32; 4], &[]);
    import_function!("_tower_fp2_bls12381_sub", TOWER_FP2_BLS12381_SUB, &[ValType::I32; 4], &[]);
    import_function!("_tower_fp2_bls12381_mul", TOWER_FP2_BLS12381_MUL, &[ValType::I32; 4], &[]);

    // secp256k1 (0x04)
    import_function!("_secp256k1_add", SECP256K1_ADD, &[ValType::I32; 2], &[]);
    import_function!("_secp256k1_decompress", SECP256K1_DECOMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_secp256k1_double", SECP256K1_DOUBLE, &[ValType::I32; 1], &[]);

    // secp256r1 (0x05)
    import_function!("_secp256r1_add", SECP256R1_ADD, &[ValType::I32; 2], &[]);
    import_function!("_secp256r1_decompress", SECP256R1_DECOMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_secp256r1_double", SECP256R1_DOUBLE, &[ValType::I32; 1], &[]);

    // bls12381 (0x06)
    import_function!("_bls12381_add", BLS12381_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bls12381_decompress", BLS12381_DECOMPRESS, &[ValType::I32; 2], &[]);
    import_function!("_bls12381_double", BLS12381_DOUBLE, &[ValType::I32; 1], &[]);

    // bn254 (0x07)
    import_function!("_bn254_add", BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bn254_double", BN254_DOUBLE, &[ValType::I32; 1], &[]);

    // uint256 (0x08)
    import_function!("_uint256_mul_mod", UINT256_MUL_MOD, &[ValType::I32; 3], &[]);
    import_function!("_uint256_x2048_mul", UINT256_X2048_MUL, &[ValType::I32; 4], &[]);

    Arc::new(import_linker)
}
