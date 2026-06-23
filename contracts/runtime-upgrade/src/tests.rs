use super::*;
use fluentbase_sdk::{address, bytes, ContractContextV1, ExitCode, B256};
use fluentbase_testing::TestingContextImpl;

struct Harness {
    sdk: TestingContextImpl,
}

impl Harness {
    fn new() -> Self {
        Self {
            sdk: TestingContextImpl::default().with_contract_context(ContractContextV1 {
                gas_limit: 120_000,
                ..Default::default()
            }),
        }
    }

    fn set_caller(&mut self, caller: Address) {
        self.sdk.context_mut().caller = caller;
    }

    fn call<I: Into<Bytes>>(&mut self, input: I) -> ExitCode {
        self.sdk = core::mem::take(&mut self.sdk).with_input(input.into());
        let storage_before_call = self.sdk.dump_storage();
        let mut app = App::new(core::mem::take(&mut self.sdk));
        let exit_code = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.main()))
        {
            Ok(_) => ExitCode::Ok,
            Err(_) => ExitCode::Panic,
        };
        self.sdk = app.sdk;
        if !exit_code.is_ok() {
            self.sdk.restore_storage(storage_before_call);
        }
        _ = self.sdk.take_output();
        exit_code
    }
}

#[test]
fn test_upgrade_to_encoding() {
    let target = address!("2222222222222222222222222222222222222222");
    let genesis_hash = B256::from([0xab; 32]);
    let genesis_version = "v1.0.0".to_string();
    // minimal valid WASM: magic bytes + version
    let wasm_bytecode = Bytes::from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00].as_ref());

    let call = UpgradeToCall::new((
        target,
        genesis_hash,
        genesis_version.clone(),
        wasm_bytecode.clone(),
    ));
    let encoded = call.encode();

    // first 4 bytes = function selector
    assert!(encoded.len() >= 4);
    println!("Encoded call data: {}", hex::encode(&encoded));

    // decode back and verify a round-trip
    let decoded = UpgradeToCall::decode(&&encoded[4..]).expect("failed to decode");
    assert_eq!(decoded.0 .0, target, "target_address mismatch");
    assert_eq!(decoded.0 .1, genesis_hash, "genesis_hash mismatch");
    assert_eq!(decoded.0 .2, genesis_version, "genesis_version mismatch");
    assert_eq!(decoded.0 .3, wasm_bytecode, "wasm_bytecode mismatch");
}

#[test]
fn test_recompile_encoding() {
    let target = address!("2222222222222222222222222222222222222222");

    let call = RecompileCall::new((target,));
    let encoded = call.encode();

    assert!(encoded.len() >= 4);
    println!("Encoded call data: {}", hex::encode(&encoded));

    let decoded = RecompileCall::decode(&&encoded[4..]).expect("failed to decode");
    assert_eq!(decoded.0 .0, target, "target_address mismatch");
}

#[test]
fn test_plan_upgrade_encoding() {
    let genesis_hash = B256::from([0xab; 32]);
    let genesis_version = "v1.0.0".to_string();
    let target_addresses = vec![
        address!("2222222222222222222222222222222222222222"),
        address!("3333333333333333333333333333333333333333"),
    ];
    let wasm_code_hashes = vec![B256::from([0x11; 32]), B256::from([0x22; 32])];
    let updater = address!("1111111111111111111111111111111111111111");

    let call = PlanUpgradeCall::new((
        genesis_hash,
        genesis_version.clone(),
        target_addresses.clone(),
        wasm_code_hashes.clone(),
        updater,
    ));
    let encoded = call.encode();

    assert!(encoded.len() >= 4);
    let decoded = PlanUpgradeCall::decode(&&encoded[4..]).expect("failed to decode");
    assert_eq!(decoded.0 .0, genesis_hash, "genesis_hash mismatch");
    assert_eq!(decoded.0 .1, genesis_version, "genesis_version mismatch");
    assert_eq!(decoded.0 .2, target_addresses, "target_addresses mismatch");
    assert_eq!(decoded.0 .3, wasm_code_hashes, "wasm_code_hashes mismatch");
    assert_eq!(decoded.0 .4, updater, "updater mismatch");
}

#[test]
fn test_upgrade_to_planned_encoding() {
    let target = address!("2222222222222222222222222222222222222222");
    let wasm_bytecode = Bytes::from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00].as_ref());

    let call = UpgradeToPlannedCall::new((target, wasm_bytecode.clone()));
    let encoded = call.encode();

    assert!(encoded.len() >= 4);
    let decoded = UpgradeToPlannedCall::decode(&&encoded[4..]).expect("failed to decode");
    assert_eq!(decoded.0 .0, target, "target_address mismatch");
    assert_eq!(decoded.0 .1, wasm_bytecode, "wasm_bytecode mismatch");
}

#[test]
fn test_upgrade_and_recompile_event_signatures_are_distinct() {
    assert_eq!(
        RuntimeUpgraded::SIGNATURE,
        "RuntimeUpgraded(address,bytes32,string,bytes32)"
    );
    assert_eq!(
        ContractRecompiled::SIGNATURE,
        "ContractRecompiled(address,bytes32)"
    );
    assert_ne!(RuntimeUpgraded::SELECTOR, ContractRecompiled::SELECTOR);
}

#[test]
fn test_planned_upgrade_rejects_same_hash_for_wrong_target() {
    let planned_target = address!("2222222222222222222222222222222222222222");
    let wrong_target = address!("3333333333333333333333333333333333333333");
    let updater = address!("1111111111111111111111111111111111111111");
    let wasm_bytecode = Bytes::from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00].as_ref());
    let wasm_code_hash = crypto_keccak256(wasm_bytecode.as_ref());

    let mut h = Harness::new();
    h.set_caller(DEFAULT_UPDATE_GENESIS_AUTH);
    let plan_call = PlanUpgradeCall::new((
        B256::from([0xab; 32]),
        "v1.0.0".to_string(),
        vec![planned_target],
        vec![wasm_code_hash],
        updater,
    ));
    assert_eq!(h.call(plan_call.encode()), ExitCode::Ok);

    h.set_caller(updater);
    let upgrade_call = UpgradeToPlannedCall::new((wrong_target, wasm_bytecode));
    assert_ne!(h.call(upgrade_call.encode()), ExitCode::Ok);
}
