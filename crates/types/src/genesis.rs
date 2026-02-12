use crate::{address, hex, Address, Bytes, B256, UNIVERSAL_TOKEN_MAGIC_BYTES, WASM_MAGIC_BYTES};

/// Address of the delegated **EVM runtime**.
///
/// Calls to this address are handled by the EVM execution engine rather than by
/// normal contract bytecode. In other words, this is a dispatcher target, not a
/// "deployed contract" in the usual sense.
pub const PRECOMPILE_EVM_RUNTIME: Address = address!("0x0000000000000000000000000000000000520001");

/// Address of the delegated **SVM runtime** (Solana VM).
///
/// This is feature-gated in some parts of the codebase (see `cfg(feature = "svm")`).
pub const PRECOMPILE_SVM_RUNTIME: Address = address!("0x0000000000000000000000000000000000520003");

/// Address of the **Wrapped ETH** contract (ERC-20 compatible representation of native ETH).
///
/// Note: Not in use
pub const PRECOMPILE_WRAPPED_ETH: Address = address!("0x0000000000000000000000000000000000520004");

/// Address of the **WebAuthn verifier** runtime.
///
/// This runtime validates WebAuthn assertions (passkeys / security keys).
pub const PRECOMPILE_WEBAUTHN_VERIFIER: Address =
    address!("0x0000000000000000000000000000000000520005");

/// Address of the **OAuth2 verifier** runtime.
///
/// Used for validating OAuth2/OpenID-style proofs and authorization assertions.
pub const PRECOMPILE_OAUTH2_VERIFIER: Address =
    address!("0x0000000000000000000000000000000000520006");

/// Address of the **Nitro verifier** runtime.
///
/// Intended for validating attestations produced by AWS Nitro Enclaves (or compatible TEEs).
pub const PRECOMPILE_NITRO_VERIFIER: Address =
    address!("0x0000000000000000000000000000000000520007");

/// Address of the delegated **Universal Token runtime**.
///
/// This runtime implements the Universal Token Standard (ERC20 + SPL),
/// and is executed by the system runtime instead of normal contract bytecode.
pub const PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME: Address =
    address!("0x0000000000000000000000000000000000520008");

/// Address of the delegated **Wasm runtime**.
///
/// Contracts that are recognized as Wasm/rWasm are dispatched here.
pub const PRECOMPILE_WASM_RUNTIME: Address = address!("0x0000000000000000000000000000000000520009");

/// EIP-2935 system contract / precompile address (as specified by the EIP).
///
/// Kept as a standalone constant so fork-activation logic can include/exclude it.
pub const PRECOMPILE_EIP2935: Address = address!("0x0000F90827F1C53a10cb7A02335B175320002935");

/// EIP-7951 system contract / precompile address (as specified by the EIP).
pub const PRECOMPILE_EIP7951: Address = address!("0x0000000000000000000000000000000000000100");

/// Constructs an EVM-style precompile address from its canonical last byte.
///
/// Ethereum precompiles live at addresses `0x000..0001` to `0x000..0011`.
/// Encoding them via the last byte keeps the mapping explicit and cheap.
const fn evm_address(value: u8) -> Address {
    Address::with_last_byte(value)
}

/// `ecrecover` precompile (EVM address 0x01).
pub const PRECOMPILE_SECP256K1_RECOVER: Address = evm_address(0x01);
/// `sha256` precompile (EVM address 0x02).
pub const PRECOMPILE_SHA256: Address = evm_address(0x02);
/// `ripemd160` precompile (EVM address 0x03).
pub const PRECOMPILE_RIPEMD160: Address = evm_address(0x03);
/// Identity precompile (copies input to output) (EVM address 0x04).
pub const PRECOMPILE_IDENTITY: Address = evm_address(0x04);
/// Big modular exponentiation precompile (EVM address 0x05).
pub const PRECOMPILE_BIG_MODEXP: Address = evm_address(0x05);
/// BN254 (a.k.a. alt_bn128) addition precompile (EVM address 0x06).
pub const PRECOMPILE_BN256_ADD: Address = evm_address(0x06);
/// BN254 (a.k.a. alt_bn128) multiplication precompile (EVM address 0x07).
pub const PRECOMPILE_BN256_MUL: Address = evm_address(0x07);
/// BN254 (a.k.a. alt_bn128) pairing check precompile (EVM address 0x08).
pub const PRECOMPILE_BN256_PAIR: Address = evm_address(0x08);
/// BLAKE2 compression function precompile (EVM address 0x09).
pub const PRECOMPILE_BLAKE2F: Address = evm_address(0x09);
/// KZG point evaluation precompile (EVM address 0x0a).
pub const PRECOMPILE_KZG_POINT_EVALUATION: Address = evm_address(0x0a);
/// BLS12-381 G1 add precompile (EVM address 0x0b).
pub const PRECOMPILE_BLS12_381_G1_ADD: Address = evm_address(0x0b);
/// BLS12-381 G1 MSM (multi-scalar multiplication) precompile (EVM address 0x0c).
pub const PRECOMPILE_BLS12_381_G1_MSM: Address = evm_address(0x0c);
/// BLS12-381 G2 add precompile (EVM address 0x0d).
pub const PRECOMPILE_BLS12_381_G2_ADD: Address = evm_address(0x0d);
/// BLS12-381 G2 MSM (multi-scalar multiplication) precompile (EVM address 0x0e).
pub const PRECOMPILE_BLS12_381_G2_MSM: Address = evm_address(0x0e);
/// BLS12-381 pairing check precompile (EVM address 0x0f).
pub const PRECOMPILE_BLS12_381_PAIRING: Address = evm_address(0x0f);
/// BLS12-381 map-to-G1 precompile (EVM address 0x10).
pub const PRECOMPILE_BLS12_381_MAP_G1: Address = evm_address(0x10);
/// BLS12-381 map-to-G2 precompile (EVM address 0x11).
pub const PRECOMPILE_BLS12_381_MAP_G2: Address = evm_address(0x11);

/// Address of the "native multicall" precompile.
///
/// Address bytes are constructed as:
/// - prefix: ASCII "R native" (8 bytes)
/// - suffix: `keccak256("multicall(bytes[])")[..4]`
///
/// The last 4 bytes double as an input selector match (see
/// `try_resolve_precompile_account_from_input`).
///
/// Note: Only for Fluent Testnet, will be removed
pub const PRECOMPILE_NATIVE_MULTICALL: Address =
    address!("0x52206e61746976650000000000000000ac9650d8");

/// The full set of addresses treated as "system precompiles" by the executor.
///
/// This list is used for routing/dispatch decisions and must remain stable
/// across nodes for consensus.
pub const PRECOMPILE_ADDRESSES: &[Address] = &[
    PRECOMPILE_BIG_MODEXP,
    PRECOMPILE_BLAKE2F,
    PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM,
    PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1,
    PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL,
    PRECOMPILE_BN256_PAIR,
    PRECOMPILE_EIP2935,
    PRECOMPILE_EIP7951,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    PRECOMPILE_EVM_RUNTIME,
    PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION,
    PRECOMPILE_NITRO_VERIFIER,
    PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256,
    PRECOMPILE_SVM_RUNTIME,
    PRECOMPILE_WASM_RUNTIME,
    PRECOMPILE_WEBAUTHN_VERIFIER,
    PRECOMPILE_WRAPPED_ETH,
];

/// Returns `true` if `address` is part of the executor's system-precompile set.
///
/// This is a pure membership check against `PRECOMPILE_ADDRESSES`.
///
/// Note: fork/spec gating (when introduced) should live here, so callers do not
/// accidentally drift by re-implementing activation logic.
pub fn is_system_precompile(address: &Address) -> bool {
    // TODO(dmitry123): Add spec check here, once we have first fork
    PRECOMPILE_ADDRESSES.contains(address)
}

/// Addresses whose execution is delegated to the **system runtime** implementation.
///
/// This is a narrower set than `PRECOMPILE_ADDRESSES`: some system contracts may
/// exist, but not be executed by the system runtime (or may be feature/fork gated).
pub const EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES: &[Address] = &[
    PRECOMPILE_BIG_MODEXP,
    PRECOMPILE_BLAKE2F,
    PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM,
    PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1,
    PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL,
    PRECOMPILE_BN256_PAIR,
    PRECOMPILE_EIP2935,
    PRECOMPILE_EIP7951,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    PRECOMPILE_EVM_RUNTIME,
    PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION,
    PRECOMPILE_NITRO_VERIFIER,
    PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256,
    // PRECOMPILE_SVM_RUNTIME,
    PRECOMPILE_WASM_RUNTIME,
    PRECOMPILE_WEBAUTHN_VERIFIER,
];

/// Returns `true` if `address` should be executed by the system runtime.
///
/// This is a separate list from `PRECOMPILE_ADDRESSES` because:
/// - some addresses may exist but be disabled until a fork activates them
/// - some addresses may be routed via different execution strategies
pub fn is_execute_using_system_runtime(address: &Address) -> bool {
    EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES.contains(address)
}

/// Addresses whose execution should be charged by the runtime.
///
/// These contracts should be compiled with `consume_fuel=true` and
/// `builtins_consume_fuel=true`.
pub const ENGINE_METERED_PRECOMPILES: &[Address] = &[
    PRECOMPILE_NITRO_VERIFIER,
    PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_WASM_RUNTIME,
    PRECOMPILE_WEBAUTHN_VERIFIER,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
];

/// Returns `true` if the contract at `address` should be charged fuel by the runtime.
pub fn is_engine_metered_precompile(address: &Address) -> bool {
    ENGINE_METERED_PRECOMPILES.contains(address)
}

/// Resolves and returns the account owner `Address` based on the provided input byte slice.
///
/// # Parameters
/// - `input`: A byte slice (`&[u8]`) used to determine the runtime owner. The function
///   inspects the beginning of the `input` slice to match specific magic byte sequences
///   associated with predefined runtime owners.
///
/// # Notes
/// - This function provides a mechanism to associate specific runtime types with accounts
///   based on their initialization input data.
pub fn resolve_precompiled_runtime_from_input(input: &[u8]) -> Address {
    if input.len() > WASM_MAGIC_BYTES.len() && input[..WASM_MAGIC_BYTES.len()] == WASM_MAGIC_BYTES {
        return PRECOMPILE_WASM_RUNTIME;
    }
    #[cfg(feature = "svm")]
    if input.len() > crate::SVM_ELF_MAGIC_BYTES.len()
        && input[..crate::SVM_ELF_MAGIC_BYTES.len()] == crate::SVM_ELF_MAGIC_BYTES
    {
        PRECOMPILE_SVM_RUNTIME
    }
    if input.len() > UNIVERSAL_TOKEN_MAGIC_BYTES.len()
        && input[..UNIVERSAL_TOKEN_MAGIC_BYTES.len()] == UNIVERSAL_TOKEN_MAGIC_BYTES
    {
        PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME
    } else {
        PRECOMPILE_EVM_RUNTIME
    }
}

/// Checks if the function call should be redirected to a native precompiled contract.
///
/// When the first four bytes of the input (function selector) match a precompile's address
/// prefix, returns the corresponding precompiled account that should handle the call.
///
/// # Arguments
/// * `input` - The complete calldata for the function call
///
/// # Returns
/// * `Some(Account)` - The precompiled account if a match is found
/// * `None` - If no matching precompile is found or in ut is too short
///
/// Note: Only for Fluent Testnet, will be removed
pub fn try_resolve_precompile_account_from_input(input: &[u8]) -> Option<Address> {
    if input.len() < 4 {
        return None;
    };
    if input[..4] == PRECOMPILE_NATIVE_MULTICALL[16..] {
        Some(PRECOMPILE_NATIVE_MULTICALL)
    } else {
        None
    }
}

/// Authority address that is allowed to update the code of arbitrary accounts.
///
/// This is the "admin" for genesis/state upgrade operations and should be
/// treated as highly privileged.
///
/// Note: Only for Fluent Testnet, will be removed
pub const UPDATE_GENESIS_AUTH: Address = address!("0xa7bf6a9168fe8a111307b7c94b8883fe02b30934");

/// Transaction calldata prefix for **genesis update** (version 1).
///
/// The executor uses this 4-byte marker to distinguish "account update" txs
/// from ordinary contract calls.
///
/// Note: Only for Fluent Testnet, will be removed
pub const UPDATE_GENESIS_PREFIX_V1: [u8; 4] = hex!("0x69bc6f64");

/// Transaction calldata prefix for **genesis update** (version 2).
///
/// Versioning allows introducing new update semantics without ambiguity.
///
/// Note: Only for Fluent Testnet, will be removed
pub const UPDATE_GENESIS_PREFIX_V2: [u8; 4] = hex!("0x69bc6f65");

#[derive(Clone)]
pub struct GenesisContract {
    /// Human-readable name of the genesis contract (used for manifests/debugging).
    pub name: &'static str,

    /// rWasm bytecode as stored in state at genesis.
    pub rwasm_bytecode: Bytes,

    /// Hash of `rwasm_bytecode`.
    ///
    /// Stored explicitly for determinism and to avoid recomputation.
    pub rwasm_bytecode_hash: B256,

    /// Address at which the contract is deployed at genesis.
    pub address: Address,
}

/// Returns `true` if `address` corresponds to a delegated runtime dispatcher.
///
/// Delegated runtimes are handled specially by the executor (they are not normal
/// bytecode-bearing contracts).
pub fn is_delegated_runtime_address(address: &Address) -> bool {
    address == &PRECOMPILE_EVM_RUNTIME
        || address == &PRECOMPILE_SVM_RUNTIME
        || address == &PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME
        || address == &PRECOMPILE_WASM_RUNTIME
}
