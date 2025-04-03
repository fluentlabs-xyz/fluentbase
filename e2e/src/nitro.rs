extern crate test;

use crate::utils::{EvmTestingContext, TxBuilder};
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::address;
use std::time::Instant;

#[test]
fn test_nitro_verifier_wasm_version() {
    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let bytecode = include_bytes!("../../contracts/nitro/lib.wasm");
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

#[test]
fn test_nitro_verifier_solidity_version() {
    let mut ctx = EvmTestingContext::default();
    let caller = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    let start = Instant::now();
    let mut total_gas = 0;

    // Step 1: Deploy CertManager.sol and NitroValidator.sol smart contracts.
    // https://github.com/base-org/nitro-validator/blob/main/src/NitroValidator.sol
    let cert_manager_bytecode = hex::decode(include_bytes!("../assets/CertManager.bin")).unwrap();
    let (cert_manager_address, gas_used) =
        ctx.deploy_evm_tx_with_nonce(caller, cert_manager_bytecode.into(), 0);
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
        ctx.deploy_evm_tx_with_nonce(caller, nitro_validator_bytecode.into(), 1);
    total_gas += gas_used;

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
    let result = TxBuilder::call(&mut ctx, caller, nitro_validator_address, None)
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
        decodeAttestationTbsCall::abi_decode_returns(result.output().unwrap(), true).unwrap();
    let input = validateAttestationCall {
        attestationTbs: parsed_attestation.attestationTbs.into(),
        signature: parsed_attestation.signature.into(),
    }
    .abi_encode();
    let result = TxBuilder::call(&mut ctx, caller, nitro_validator_address, None)
        .gas_limit(70_000_000)
        .input(input.into())
        .timestamp(1695050165) // ensure correct block timestamp to match certificate time window.
        .exec();
    if !result.is_success() {
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
