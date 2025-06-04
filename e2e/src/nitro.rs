extern crate test;

use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{address, Address, U256};
use fluentbase_sdk_testing::{try_print_utf8_error, EvmTestingContext, TxBuilder};
use std::time::Instant;

#[ignore] // TODO(khasan) nitro has floats for some reason, investigate why and how to remove them
#[test]
fn test_nitro_verifier_wasm_version() {
    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let bytecode = fluentbase_contracts_nitro::BUILD_OUTPUT
        .wasm_bytecode
        .to_vec();
    let mut ctx = EvmTestingContext::default();
    let address = ctx.deploy_evm_tx(caller, bytecode.into());

    let start = Instant::now();
    let mut total_gas = 0;

    let attestation_doc: Vec<u8> = hex::decode(include_bytes!(
        "../../contracts/nitro/attestation-example.hex"
    ))
    .unwrap()
    .into();
    let result = TxBuilder::call(&mut ctx, caller, address, None)
        .input(attestation_doc.into())
        .gas_limit(1_000_000_000)
        .exec();
    if !result.is_success() {
        panic!("attestation verification failed, result: {:?}", result);
    }
    total_gas += result.gas_used();
    println!(
        "NitroVerifier.wasm: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
}

#[ignore] // slow because it runs in the interpreter by default (wasmtime is not enabled by default)
#[test]
fn test_nitro_verifier_solidity_version() {
    let mut ctx = EvmTestingContext::default();
    ctx.cfg.disable_rwasm_proxy = true;

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
    let attestation_bytes = hex::decode(include_bytes!(
        "../../contracts/nitro/attestation-example.hex"
    ))
    .unwrap();
    let input = decodeAttestationTbsCall {
        attestation: attestation_bytes.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, OWNER_ADDRESS, nitro_validator_address, None)
        .input(input.into())
        .exec();
    if !result.is_success() {
        panic!("decode attestation tbs call failed, result: {:?}", result);
    }
    total_gas += result.gas_used();

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
    let result = TxBuilder::call(&mut ctx, OWNER_ADDRESS, nitro_validator_address, None)
        .gas_limit(70_000_000)
        .input(input.into())
        .timestamp(1695050165) // ensure correct block timestamp to match certificate time window.
        .exec();
    if !result.is_success() {
        try_print_utf8_error(result.output().cloned().unwrap_or_default().as_ref());
        panic!("validate attestation call failed, result: {:?}", result);
    }
    total_gas += result.gas_used();

    // Step 4: Output the total execution time and cumulative gas used.
    println!(
        "NitroVerifier.sol: time={:.2?}, gas={}",
        start.elapsed(),
        total_gas
    );
}
