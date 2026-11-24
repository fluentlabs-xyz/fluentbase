use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolValue};
use fluentbase_sdk::{address, Address, Bytes, PRECOMPILE_NITRO_VERIFIER, U256};
use fluentbase_testing::{try_print_utf8_error, EvmTestingContext, TxBuilder};
use revm::{context::result::ExecutionResult, primitives::hardfork::SpecId};
use std::time::Instant;

/// A real attestation document and the creation time it carries, in seconds. Every certificate
/// of its chain is valid at that time; the leaf expires three hours later.
const ATTESTATION_EXAMPLE: &[u8] = include_bytes!("../../../contracts/nitro/testdata/sample.bin");
const ATTESTATION_TIMESTAMP: u64 = 1_695_049_410;

/// Gas forwarded to the verifier in the rejection tests: more than a verification needs, so a
/// frame that halts instead of reverting shows up as all of it being spent.
const REJECTION_GAS_LIMIT: u64 = 30_000_000;

/// A CBOR byte string: the shortest head for `data`, then `data`.
fn cbor_bstr(data: &[u8]) -> Vec<u8> {
    let mut out = match data.len() {
        len @ 0..=23 => vec![0x40 | len as u8],
        len @ 24..=0xff => vec![0x58, len as u8],
        len => {
            let mut head = vec![0x59];
            head.extend_from_slice(&u16::try_from(len).unwrap().to_be_bytes());
            head
        }
    };
    out.extend_from_slice(data);
    out
}

/// A COSE_Sign1 envelope with an empty unprotected header.
fn cose_envelope(protected: &[u8], payload: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut out = vec![0x84];
    out.extend(cbor_bstr(protected));
    out.push(0xa0);
    out.extend(cbor_bstr(payload));
    out.extend(cbor_bstr(signature));
    out
}

/// `depth` arrays of one item, each inside the previous one, around the integer 0.
fn nested_arrays(depth: usize) -> Vec<u8> {
    let mut out = vec![0x81; depth];
    out.push(0x00);
    out
}

/// A COSE header that carries a counter signature whose own protected header carries one, and
/// so on, `depth` times: `{7: [h'<header>', {}, h'']}`.
fn nested_counter_signatures(depth: usize) -> Vec<u8> {
    let mut header = vec![0xa0];
    for _ in 0..depth {
        let mut outer = vec![0xa1, 0x07, 0x83];
        outer.extend(cbor_bstr(&header));
        outer.extend_from_slice(&[0xa0, 0x40]);
        header = outer;
    }
    header
}

/// Reason carried by a Solidity `Error(string)` revert payload.
fn revert_reason(output: &[u8]) -> &str {
    assert_eq!(&output[..4], &[0x08, 0xc3, 0x79, 0xa0], "not Error(string)");
    let len = u32::from_be_bytes(output[64..68].try_into().unwrap()) as usize;
    core::str::from_utf8(&output[68..68 + len]).unwrap()
}

/// Malformed input must revert with a reason and return the unused gas. Each of these inputs
/// used to end the frame abnormally instead: the oversized one halted it, the short signature
/// panicked the guest after the whole certificate chain had been verified, and the nested ones
/// overflowed the guest stack inside a recursive CBOR parser. A halt burns all the gas forwarded.
#[test]
fn test_nitro_verifier_reverts_on_malformed_input() {
    const ES384_PROTECTED: [u8; 4] = [0xa1, 0x01, 0x38, 0x22];
    const SIGNATURE_LEN: usize = 96;

    let payload_end = ATTESTATION_EXAMPLE.len() - (2 + SIGNATURE_LEN);
    // The envelope is `84 44 a1013822 a0 59 <len:2> <payload> 58 60 <signature>`.
    let payload = &ATTESTATION_EXAMPLE[10..payload_end];
    let signature = &ATTESTATION_EXAMPLE[payload_end + 2..];
    assert_eq!(
        cose_envelope(&ES384_PROTECTED, payload, signature),
        ATTESTATION_EXAMPLE
    );

    let cases: [(&str, Vec<u8>, &str); 6] = [
        (
            "input above the size cap",
            vec![0u8; 16 * 1024 + 1],
            "nitro: input too large",
        ),
        (
            "signature of 95 bytes",
            cose_envelope(&ES384_PROTECTED, payload, &signature[..SIGNATURE_LEN - 1]),
            "nitro: invalid signature length",
        ),
        (
            "nested arrays",
            nested_arrays(2000),
            "nitro: not a CoseSign1",
        ),
        (
            "counter signatures nested in the protected header",
            cose_envelope(&nested_counter_signatures(1900), &[], signature),
            "nitro: unsupported COSE protected header",
        ),
        (
            "nested payload",
            cose_envelope(&ES384_PROTECTED, &nested_arrays(2000), signature),
            "nitro: malformed attestation document",
        ),
        (
            "payload nested one level too deep",
            cose_envelope(&ES384_PROTECTED, &nested_arrays(3), signature),
            "nitro: malformed attestation document",
        ),
    ];

    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(caller, U256::from(500_000_000_000u128));

    for (name, input, expected_reason) in cases {
        let result = TxBuilder::call(&mut ctx, PRECOMPILE_NITRO_VERIFIER)
            .caller(caller)
            .input(input.into())
            .gas_limit(REJECTION_GAS_LIMIT)
            .timestamp(ATTESTATION_TIMESTAMP)
            .exec();
        let ExecutionResult::Revert { output, .. } = &result else {
            panic!("{name}: expected a revert, got {result:?}");
        };
        assert_eq!(revert_reason(output), expected_reason, "{name}");
        assert!(
            result.tx_gas_used() < REJECTION_GAS_LIMIT / 10,
            "{name}: rejection used {} gas",
            result.tx_gas_used()
        );
    }

    // The verifier still accepts a valid document afterwards.
    let result = TxBuilder::call(&mut ctx, PRECOMPILE_NITRO_VERIFIER)
        .caller(caller)
        .input(ATTESTATION_EXAMPLE.to_vec().into())
        .gas_limit(100_000_000)
        .timestamp(ATTESTATION_TIMESTAMP)
        .exec();
    assert!(result.is_success(), "valid document: {result:?}");
}

#[test]
fn test_nitro_verifier_precompiled_version() {
    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(caller, U256::from(500_000_000_000u128));
    let start = Instant::now();
    let mut total_gas = 0;
    let attestation_doc: Vec<u8> = ATTESTATION_EXAMPLE.to_vec();
    let result = TxBuilder::call(&mut ctx, PRECOMPILE_NITRO_VERIFIER)
        .caller(caller)
        .input(attestation_doc.into())
        .gas_limit(100_000_000)
        .timestamp(ATTESTATION_TIMESTAMP)
        .exec();
    if !result.is_success() {
        panic!("attestation verification failed, result: {:?}", result);
    }
    total_gas += result.tx_gas_used();

    let (_module_id, timestamp, pcrs, _public_key, _user_data, _nonce) =
        <(String, u64, Vec<Bytes>, Bytes, Bytes, Bytes)>::abi_decode_params(
            result.output().unwrap(),
        )
        .unwrap();
    assert_eq!(timestamp / 1000, ATTESTATION_TIMESTAMP);
    assert_eq!(pcrs.len(), 16);
    assert!(pcrs.iter().all(|pcr| pcr.len() == 48));

    println!(
        "NitroVerifier.wasm: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
}

#[test]
fn test_nitro_verifier_original_version() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.disabled_rwasm = true;
    ctx.cfg.spec = SpecId::PRAGUE;

    const OWNER_ADDRESS: Address = Address::ZERO;
    ctx.add_balance(OWNER_ADDRESS, U256::from(1e18));

    let start = Instant::now();
    let mut total_gas = 0;

    // Step 1: Deploy CertManager.sol and NitroValidator.sol smart contracts.
    // https://github.com/base-org/nitro-validator/blob/main/src/NitroValidator.sol
    let cert_manager_bytecode = hex::decode(include_bytes!("../assets/CertManager.bin")).unwrap();
    let (cert_manager_address, gas_used) =
        ctx.deploy_evm_tx_with_gas(OWNER_ADDRESS, cert_manager_bytecode.into());
    total_gas += gas_used;
    let mut nitro_validator_bytecode =
        hex::decode(include_bytes!("../assets/NitroValidator.bin")).unwrap();
    let constructor_args = hex::decode(format!(
        "000000000000000000000000{}",
        cert_manager_address.to_string().get(2..).unwrap(),
    ))
    .unwrap();
    nitro_validator_bytecode.extend(constructor_args);
    let (nitro_validator_address, gas_used) =
        ctx.deploy_evm_tx_with_gas(OWNER_ADDRESS, nitro_validator_bytecode.into());
    total_gas += gas_used;

    println!("cert_manager_address={}", cert_manager_address);
    println!("nitro_validator_address={}", nitro_validator_address);

    // Step 2: Decode the attestation blob into "to-be-signed" and "signature" via
    // decodeAttestationTbs().
    sol! {
        function decodeAttestationTbs(bytes memory attestation) external pure returns (bytes memory attestationTbs, bytes memory signature);
    }
    let attestation_bytes: Vec<u8> = ATTESTATION_EXAMPLE.to_vec();
    let input = decodeAttestationTbsCall {
        attestation: attestation_bytes.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, nitro_validator_address)
        .caller(OWNER_ADDRESS)
        .input(input.into())
        .exec();
    if !result.is_success() {
        panic!("decode attestation tbs call failed, result: {:?}", result);
    }
    total_gas += result.tx_gas_used();

    // Step 3: Validate the attestation document using the decoded values.
    sol! {
        type CborElement is uint256;
        struct Ptrs {
            CborElement moduleID;
            uint64 timestamp;
            CborElement digest;
            CborElement[] pcrs;
            CborElement cert;
            CborElement[] cabundle;
            CborElement publicKey;
            CborElement userData;
            CborElement nonce;
        }
        function validateAttestation(bytes memory attestationTbs, bytes memory signature) public returns (Ptrs memory);
    }
    let parsed_attestation =
        decodeAttestationTbsCall::abi_decode_returns_validate(result.output().unwrap()).unwrap();
    let input = validateAttestationCall {
        attestationTbs: parsed_attestation.attestationTbs.into(),
        signature: parsed_attestation.signature.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, nitro_validator_address)
        .caller(OWNER_ADDRESS)
        .gas_limit(70_000_000)
        .input(input.into())
        .timestamp(1695050165) // ensure correct block timestamp to match certificate time window.
        .exec();
    if !result.is_success() {
        try_print_utf8_error(result.output().cloned().unwrap_or_default().as_ref());
        panic!("validate attestation call failed, result: {:?}", result);
    }
    total_gas += result.tx_gas_used();

    // Step 4: Output the total execution time and cumulative gas used.
    println!(
        "NitroVerifier.sol: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
}

/// The same Solidity validator, executed by the Fluent EVM. It reverts after spending 99.25 Mgas
/// of the 100 Mgas transaction cap: plain revm does exactly the same under Osaka gas rules, which
/// is what Fluent applies, while `test_nitro_verifier_original_version` pins Prague.
#[test]
#[ignore = "the Solidity validator does not fit the 100 Mgas transaction cap under Osaka gas rules"]
fn test_nitro_verifier_solidity_version() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.cfg.spec = SpecId::PRAGUE;

    const OWNER_ADDRESS: Address = Address::ZERO;
    ctx.add_balance(OWNER_ADDRESS, U256::from(1e18));

    let start = Instant::now();
    let mut total_gas = 0;

    // Step 1: Deploy CertManager.sol and NitroValidator.sol smart contracts.
    // https://github.com/base-org/nitro-validator/blob/main/src/NitroValidator.sol
    let cert_manager_bytecode = hex::decode(include_bytes!("../assets/CertManager.bin")).unwrap();
    let (cert_manager_address, gas_used) =
        ctx.deploy_evm_tx_with_gas(OWNER_ADDRESS, cert_manager_bytecode.into());
    total_gas += gas_used;
    let mut nitro_validator_bytecode =
        hex::decode(include_bytes!("../assets/NitroValidator.bin")).unwrap();
    let constructor_args = hex::decode(format!(
        "000000000000000000000000{}",
        cert_manager_address.to_string().get(2..).unwrap(),
    ))
    .unwrap();
    nitro_validator_bytecode.extend(constructor_args);
    let (nitro_validator_address, gas_used) =
        ctx.deploy_evm_tx_with_gas(OWNER_ADDRESS, nitro_validator_bytecode.into());
    total_gas += gas_used;

    println!("cert_manager_address={}", cert_manager_address);
    println!("nitro_validator_address={}", nitro_validator_address);

    // Step 2: Decode the attestation blob into "to-be-signed" and "signature" via
    // decodeAttestationTbs().
    sol! {
        function decodeAttestationTbs(bytes memory attestation) external pure returns (bytes memory attestationTbs, bytes memory signature);
    }
    let attestation_bytes: Vec<u8> = ATTESTATION_EXAMPLE.to_vec();
    let input = decodeAttestationTbsCall {
        attestation: attestation_bytes.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, nitro_validator_address)
        .caller(OWNER_ADDRESS)
        .input(input.into())
        .exec();
    if !result.is_success() {
        panic!("decode attestation tbs call failed, result: {:?}", result);
    }
    total_gas += result.tx_gas_used();

    // Step 3: Validate the attestation document using the decoded values.
    sol! {
        type CborElement is uint256;
        struct Ptrs {
            CborElement moduleID;
            uint64 timestamp;
            CborElement digest;
            CborElement[] pcrs;
            CborElement cert;
            CborElement[] cabundle;
            CborElement publicKey;
            CborElement userData;
            CborElement nonce;
        }
        function validateAttestation(bytes memory attestationTbs, bytes memory signature) public returns (Ptrs memory);
    }
    let parsed_attestation =
        decodeAttestationTbsCall::abi_decode_returns_validate(result.output().unwrap()).unwrap();
    let input = validateAttestationCall {
        attestationTbs: parsed_attestation.attestationTbs.into(),
        signature: parsed_attestation.signature.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, nitro_validator_address)
        .caller(OWNER_ADDRESS)
        .gas_limit(100_000_000)
        .input(input.into())
        .timestamp(1695050165) // ensure correct block timestamp to match certificate time window.
        .exec();
    if !result.is_success() {
        try_print_utf8_error(result.output().cloned().unwrap_or_default().as_ref());
        panic!("validate attestation call failed, result: {:?}", result);
    }
    total_gas += result.tx_gas_used();

    // Step 4: Output the total execution time and cumulative gas used.
    println!(
        "NitroVerifier.sol: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
}
