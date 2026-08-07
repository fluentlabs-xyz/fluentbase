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

    /// Inspect or drive contract state directly, bypassing the router.
    fn with_app<R>(&mut self, f: impl FnOnce(&mut App<TestingContextImpl>) -> R) -> R {
        let mut app = App::new(core::mem::take(&mut self.sdk));
        let result = f(&mut app);
        self.sdk = app.sdk;
        result
    }

    fn owner(&mut self) -> Address {
        self.with_app(|app| app.owner_accessor().get(&app.sdk))
    }

    fn planned_updater(&mut self) -> Address {
        self.with_app(|app| app.planned_updater_accessor().get(&app.sdk))
    }

    fn planned_genesis(&mut self) -> (B256, String) {
        self.with_app(|app| {
            (
                app.planned_genesis_hash_accessor().get(&app.sdk),
                app.planned_genesis_version_accessor().get(&app.sdk),
            )
        })
    }

    fn planned_len(&mut self) -> u64 {
        self.with_app(|app| app.planned_wasm_hashes_accessor().len(&app.sdk))
    }

    fn has_planned(&mut self, target_address: Address, wasm_code_hash: B256) -> bool {
        self.with_app(|app| app.has_planned_upgrade(target_address, wasm_code_hash))
    }

    /// Consume a planned pair the way a successful `upgradeToPlanned` does. The install path
    /// itself needs a real runtime syscall, which the testing context does not provide.
    fn consume_planned(&mut self, target_address: Address, wasm_code_hash: B256) {
        self.with_app(|app| app.remove_planned_upgrade(target_address, wasm_code_hash));
    }
}

const OWNER: Address = DEFAULT_UPDATE_GENESIS_AUTH;
const NEW_OWNER: Address = address!("4444444444444444444444444444444444444444");
const UPDATER: Address = address!("1111111111111111111111111111111111111111");
const TARGET_A: Address = address!("2222222222222222222222222222222222222222");
const TARGET_B: Address = address!("3333333333333333333333333333333333333333");

/// Minimal valid WASM (magic bytes + version) with a trailing byte to vary the hash.
fn wasm_for(target_address: Address) -> Bytes {
    Bytes::from(vec![
        0x00,
        0x61,
        0x73,
        0x6d,
        0x01,
        0x00,
        0x00,
        0x00,
        target_address.0[0],
    ])
}

fn plan_two_targets(h: &mut Harness) -> (B256, B256) {
    let hash_a = crypto_keccak256(wasm_for(TARGET_A).as_ref());
    let hash_b = crypto_keccak256(wasm_for(TARGET_B).as_ref());

    h.set_caller(OWNER);
    let plan_call = PlanUpgradeCall::new((
        B256::from([0xab; 32]),
        "v1.0.0".to_string(),
        vec![TARGET_A, TARGET_B],
        vec![hash_a, hash_b],
        UPDATER,
    ));
    assert_eq!(h.call(plan_call.encode()), ExitCode::Ok);

    (hash_a, hash_b)
}

fn assert_plan_is_cancelled(h: &mut Harness, hash_a: B256, hash_b: B256) {
    assert_eq!(
        h.planned_updater(),
        Address::ZERO,
        "updater still delegated"
    );
    assert_eq!(h.planned_len(), 0, "planned pairs survived");
    assert!(!h.has_planned(TARGET_A, hash_a));
    assert!(!h.has_planned(TARGET_B, hash_b));
    assert_eq!(
        h.planned_genesis(),
        (B256::ZERO, String::new()),
        "genesis metadata survived"
    );

    // The previously delegated updater can no longer consume any leftover of the old plan.
    h.set_caller(UPDATER);
    for (target, wasm) in [
        (TARGET_A, wasm_for(TARGET_A)),
        (TARGET_B, wasm_for(TARGET_B)),
    ] {
        let upgrade_call = UpgradeToPlannedCall::new((target, wasm));
        assert_eq!(
            h.call(upgrade_call.encode()),
            ExitCode::Panic,
            "delegated updater still authorized for {target}"
        );
    }
}

fn assert_plan_is_intact(h: &mut Harness, hash_a: B256, hash_b: B256) {
    assert_eq!(h.planned_updater(), UPDATER);
    assert_eq!(h.planned_len(), 2);
    assert!(h.has_planned(TARGET_A, hash_a));
    assert!(h.has_planned(TARGET_B, hash_b));
    assert_eq!(
        h.planned_genesis(),
        (B256::from([0xab; 32]), "v1.0.0".to_string())
    );
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
fn test_upgrade_evm_to_encoding() {
    let target = address!("2222222222222222222222222222222222222222");
    let genesis_hash = B256::from([0xab; 32]);
    let genesis_version = "v1.0.0".to_string();
    let evm_bytecode = bytes!("6001600055");

    let call = UpgradeEvmToCall::new((
        target,
        genesis_hash,
        genesis_version.clone(),
        evm_bytecode.clone(),
    ));
    let encoded = call.encode();

    let decoded = UpgradeEvmToCall::decode(&&encoded[4..]).expect("failed to decode");
    assert_eq!(decoded.0 .0, target);
    assert_eq!(decoded.0 .1, genesis_hash);
    assert_eq!(decoded.0 .2, genesis_version);
    assert_eq!(decoded.0 .3, evm_bytecode);
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
fn test_non_owner_plan_upgrade_rejects_count_larger_than_array_body() {
    let call = PlanUpgradeCall::new((
        B256::from([0xab; 32]),
        "v1.0.0".to_string(),
        vec![address!("2222222222222222222222222222222222222222")],
        vec![B256::from([0x11; 32])],
        address!("1111111111111111111111111111111111111111"),
    ));
    let mut encoded = call.encode().to_vec();

    // The third planUpgrade argument is target_addresses. Its ABI head starts after the selector,
    // genesis hash, and genesis version offset. Replace its body count while keeping the body tiny.
    let target_addresses_head = 4 + 64;
    let target_addresses_offset = u32::from_be_bytes(
        encoded[target_addresses_head + 28..target_addresses_head + 32]
            .try_into()
            .expect("target_addresses offset must be a u32"),
    ) as usize;
    let target_addresses_length = 4 + target_addresses_offset;
    encoded[target_addresses_length + 28..target_addresses_length + 32]
        .copy_from_slice(&u32::MAX.to_be_bytes());

    assert!(PlanUpgradeCall::decode(&&encoded[4..]).is_err());

    let mut h = Harness::new();
    assert_eq!(h.call(Bytes::from(encoded)), ExitCode::Panic);
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
fn test_upgrade_plan_cancelled_event_signature_is_distinct() {
    assert_eq!(
        UpgradePlanCancelled::SIGNATURE,
        "UpgradePlanCancelled(bytes32,address,address[],bytes32[])"
    );
    assert_ne!(UpgradePlanCancelled::SELECTOR, UpgradePlanned::SELECTOR);
}

#[test]
fn test_change_owner_revokes_planned_upgrade() {
    let mut h = Harness::new();
    let (hash_a, hash_b) = plan_two_targets(&mut h);
    assert_plan_is_intact(&mut h, hash_a, hash_b);

    h.set_caller(OWNER);
    assert_eq!(
        h.call(ChangeOwnerCall::new((NEW_OWNER,)).encode()),
        ExitCode::Ok
    );
    assert_eq!(h.owner(), NEW_OWNER);

    assert_plan_is_cancelled(&mut h, hash_a, hash_b);
}

#[test]
fn test_renounce_ownership_revokes_planned_upgrade() {
    let mut h = Harness::new();
    let (hash_a, hash_b) = plan_two_targets(&mut h);

    h.set_caller(OWNER);
    assert_eq!(
        h.call(RenounceOwnershipCall::new(()).encode()),
        ExitCode::Ok
    );
    assert_eq!(h.owner(), SYSTEM_ADDRESS);

    assert_plan_is_cancelled(&mut h, hash_a, hash_b);
}

#[test]
fn test_partially_consumed_plan_is_revoked_on_owner_change() {
    let mut h = Harness::new();
    let (hash_a, hash_b) = plan_two_targets(&mut h);

    // The delegated updater installs the first pair; the second one is still pending.
    h.consume_planned(TARGET_A, hash_a);
    assert_eq!(h.planned_len(), 1);
    assert!(!h.has_planned(TARGET_A, hash_a));
    assert!(h.has_planned(TARGET_B, hash_b));

    h.set_caller(OWNER);
    assert_eq!(
        h.call(ChangeOwnerCall::new((NEW_OWNER,)).encode()),
        ExitCode::Ok
    );

    assert_plan_is_cancelled(&mut h, hash_a, hash_b);
}

#[test]
fn test_new_owner_plans_without_inheriting_stale_entries() {
    let mut h = Harness::new();
    let (hash_a, hash_b) = plan_two_targets(&mut h);

    h.set_caller(OWNER);
    assert_eq!(
        h.call(ChangeOwnerCall::new((NEW_OWNER,)).encode()),
        ExitCode::Ok
    );

    // The old owner cannot plan anymore, and the new owner's plan covers only its own pair.
    let new_updater = address!("5555555555555555555555555555555555555555");
    let fresh_plan = PlanUpgradeCall::new((
        B256::from([0xcd; 32]),
        "v2.0.0".to_string(),
        vec![TARGET_A],
        vec![hash_a],
        new_updater,
    ));
    assert_eq!(h.call(fresh_plan.encode()), ExitCode::Panic);

    h.set_caller(NEW_OWNER);
    assert_eq!(h.call(fresh_plan.encode()), ExitCode::Ok);

    assert_eq!(h.planned_len(), 1);
    assert!(h.has_planned(TARGET_A, hash_a));
    assert!(!h.has_planned(TARGET_B, hash_b));
    assert_eq!(h.planned_updater(), new_updater);
    assert_eq!(
        h.planned_genesis(),
        (B256::from([0xcd; 32]), "v2.0.0".to_string())
    );

    // The updater delegated by the previous owner has no authority under the new plan.
    h.set_caller(UPDATER);
    let upgrade_call = UpgradeToPlannedCall::new((TARGET_A, wasm_for(TARGET_A)));
    assert_eq!(h.call(upgrade_call.encode()), ExitCode::Panic);
}

#[test]
fn test_reverted_ownership_transition_keeps_plan_intact() {
    let mut h = Harness::new();
    let (hash_a, hash_b) = plan_two_targets(&mut h);

    // Zero-address transfer is rejected after the plan would otherwise have been cancelled.
    h.set_caller(OWNER);
    assert_eq!(
        h.call(ChangeOwnerCall::new((Address::ZERO,)).encode()),
        ExitCode::Panic
    );
    assert_eq!(h.owner(), Address::ZERO, "owner slot must stay untouched");
    assert_plan_is_intact(&mut h, hash_a, hash_b);

    // Neither can a non-owner drop the plan via a failed transition.
    h.set_caller(UPDATER);
    assert_eq!(
        h.call(ChangeOwnerCall::new((NEW_OWNER,)).encode()),
        ExitCode::Panic
    );
    assert_eq!(
        h.call(RenounceOwnershipCall::new(()).encode()),
        ExitCode::Panic
    );
    assert_plan_is_intact(&mut h, hash_a, hash_b);

    // The plan is still usable by its delegated updater.
    assert_eq!(h.planned_updater(), UPDATER);
}

#[test]
fn test_ownership_transition_without_plan_emits_no_cancellation() {
    let mut h = Harness::new();

    h.set_caller(OWNER);
    _ = h.sdk.take_logs();
    assert_eq!(
        h.call(ChangeOwnerCall::new((NEW_OWNER,)).encode()),
        ExitCode::Ok
    );

    let logs = h.sdk.take_logs();
    assert_eq!(logs.len(), 1, "expected only OwnerChanged");
    assert_eq!(logs[0].1[0].0, OwnerChanged::SELECTOR);
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

/// Log-data ABI vectors.
///
/// Solidity encodes event data with top-level argument semantics (`abi.encode(a, b, ...)`), so
/// dynamic non-indexed fields must NOT be wrapped in an outer tuple offset. Every expected vector
/// below is written out word by word, independently of the encoder, and cross-checked against
/// `alloy-sol-types` — a standard log decoder — so a regression to tuple-value encoding fails here.
mod log_data_abi {
    use super::*;
    use alloy_sol_types::SolEvent;
    use fluentbase_sdk::U256;

    /// The same events declared in Solidity. Names must match the Rust ones: the decoder checks
    /// `topics[0]` against the signature hash it derives itself.
    mod sol_abi {
        alloy_sol_types::sol! {
            event RuntimeUpgraded(
                address indexed target_address,
                bytes32 indexed genesis_hash,
                string genesis_version,
                bytes32 code_hash
            );

            event UpgradePlanned(
                bytes32 indexed genesis_hash,
                string genesis_version,
                address[] target_addresses,
                bytes32[] wasm_code_hashes,
                address updater
            );

            event OwnerChanged(address new_owner);

            event Mixed(address indexed who, uint256 amount, string note, address tail);
        }
    }

    /// A non-indexed field placed after a dynamic one: its head word must stay in place instead of
    /// sliding behind a wrapper offset.
    #[derive(Event)]
    struct Mixed {
        #[indexed]
        who: Address,
        amount: U256,
        note: String,
        tail: Address,
    }

    /// Emits a single event into a throwaway context and returns `(topics, data)`.
    fn emitted(emit: impl FnOnce(&mut TestingContextImpl)) -> (Vec<B256>, Bytes) {
        let mut sdk = TestingContextImpl::default();
        emit(&mut sdk);
        let mut logs = sdk.take_logs();
        assert_eq!(logs.len(), 1, "expected exactly one log");
        let (data, topics) = logs.remove(0);
        (topics, data)
    }

    /// Concatenates 32-byte words into the expected `data` blob.
    fn words(words: &[[u8; 32]]) -> Vec<u8> {
        words.concat()
    }

    fn word_u64(value: u64) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&value.to_be_bytes());
        word
    }

    fn word_address(address: Address) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[12..].copy_from_slice(address.as_slice());
        word
    }

    fn word_bytes32(value: B256) -> [u8; 32] {
        value.0
    }

    /// Right-pads a string to a single 32-byte word (all fixtures here are shorter than 32 bytes).
    fn word_utf8(text: &str) -> [u8; 32] {
        assert!(text.len() <= 32, "fixture string must fit one word");
        let mut word = [0u8; 32];
        word[..text.len()].copy_from_slice(text.as_bytes());
        word
    }

    #[test]
    fn runtime_upgraded_data_matches_solidity_argument_encoding() {
        let target_address = address!("1111111111111111111111111111111111111111");
        let genesis_hash = B256::repeat_byte(0x22);
        let genesis_version = "v1.2.3".to_string();
        let code_hash = B256::repeat_byte(0x33);

        let (topics, data) = emitted(|sdk| {
            RuntimeUpgraded {
                target_address,
                genesis_hash,
                genesis_version: genesis_version.clone(),
                code_hash,
            }
            .emit(sdk)
            .unwrap()
        });

        // head: [offset(genesis_version), code_hash], tail: [len, utf8]
        let expected = words(&[
            word_u64(0x40),
            word_bytes32(code_hash),
            word_u64(genesis_version.len() as u64),
            word_utf8(&genesis_version),
        ]);
        assert_eq!(hex::encode(&data), hex::encode(&expected));

        let decoded =
            sol_abi::RuntimeUpgraded::decode_raw_log(topics, &data).expect("standard decoder");
        assert_eq!(decoded.target_address, target_address);
        assert_eq!(decoded.genesis_hash, genesis_hash);
        assert_eq!(decoded.genesis_version, genesis_version);
        assert_eq!(decoded.code_hash, code_hash);
    }

    #[test]
    fn upgrade_planned_data_matches_solidity_argument_encoding() {
        let genesis_hash = B256::repeat_byte(0x44);
        let genesis_version = "planned".to_string();
        let target_a = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let target_b = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let hash_a = B256::repeat_byte(0xcc);
        let hash_b = B256::repeat_byte(0xdd);
        let updater = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

        let (topics, data) = emitted(|sdk| {
            UpgradePlanned {
                genesis_hash,
                genesis_version: genesis_version.clone(),
                target_addresses: vec![target_a, target_b],
                wasm_code_hashes: vec![hash_a, hash_b],
                updater,
            }
            .emit(sdk)
            .unwrap()
        });

        // head: [off(version)=0x80, off(targets)=0xc0, off(hashes)=0x120, updater]
        let expected = words(&[
            word_u64(0x80),
            word_u64(0xc0),
            word_u64(0x120),
            word_address(updater),
            word_u64(genesis_version.len() as u64),
            word_utf8(&genesis_version),
            word_u64(2),
            word_address(target_a),
            word_address(target_b),
            word_u64(2),
            word_bytes32(hash_a),
            word_bytes32(hash_b),
        ]);
        assert_eq!(hex::encode(&data), hex::encode(&expected));

        let decoded =
            sol_abi::UpgradePlanned::decode_raw_log(topics, &data).expect("standard decoder");
        assert_eq!(decoded.genesis_hash, genesis_hash);
        assert_eq!(decoded.genesis_version, genesis_version);
        assert_eq!(decoded.target_addresses, vec![target_a, target_b]);
        assert_eq!(decoded.wasm_code_hashes, vec![hash_a, hash_b]);
        assert_eq!(decoded.updater, updater);
    }

    #[test]
    fn mixed_static_and_dynamic_data_matches_solidity_argument_encoding() {
        let who = address!("1010101010101010101010101010101010101010");
        let amount = U256::from(0x1234u64);
        let note = "mixed".to_string();
        let tail = address!("2020202020202020202020202020202020202020");

        let (topics, data) = emitted(|sdk| {
            Mixed {
                who,
                amount,
                note: note.clone(),
                tail,
            }
            .emit(sdk)
            .unwrap()
        });

        // head: [amount, off(note)=0x60, tail], tail: [len, utf8]
        let expected = words(&[
            word_u64(0x1234),
            word_u64(0x60),
            word_address(tail),
            word_u64(note.len() as u64),
            word_utf8(&note),
        ]);
        assert_eq!(hex::encode(&data), hex::encode(&expected));

        let decoded = sol_abi::Mixed::decode_raw_log(topics, &data).expect("standard decoder");
        assert_eq!(decoded.who, who);
        assert_eq!(decoded.amount, amount);
        assert_eq!(decoded.note, note);
        assert_eq!(decoded.tail, tail);
    }

    #[test]
    fn fully_static_data_has_no_offset_word() {
        let new_owner = address!("3030303030303030303030303030303030303030");

        let (topics, data) = emitted(|sdk| OwnerChanged { new_owner }.emit(sdk).unwrap());

        assert_eq!(
            hex::encode(&data),
            hex::encode(words(&[word_address(new_owner)]))
        );

        let decoded =
            sol_abi::OwnerChanged::decode_raw_log(topics, &data).expect("standard decoder");
        assert_eq!(decoded.new_owner, new_owner);
    }
}
