use std::path::PathBuf;

use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall, SolValue};
use fluentbase_genesis_bootstrap::{
    artifacts, bootstrap,
    bootstrap::{BLS_VERIFIER_ADDR, CHAIN_CONFIG_ADDR, GOVERNANCE_ADDR, LIVENESS_SLASHING_ADDR,
                PredeployState, STAKING_ADDR, STAKING_POOL_ADDR, STAKING_TOKEN_ADDR,
                SYSTEM_REWARD_ADDR},
    keys,
};
use fluentbase_testing::EvmTestingContext;

const SMOKE_CHAIN_ID: u64 = 2026;
const SMOKE_MNEMONIC: &str =
    "test test test test test test test test test test test junk";

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("contracts")
}

fn run_bootstrap(peers: u32) -> (keys::KeySet, PredeployState) {
    let key_set = keys::derive(SMOKE_MNEMONIC, peers, SMOKE_CHAIN_ID).unwrap();
    let arts = artifacts::load(&contracts_dir()).unwrap();
    let state = bootstrap::run(&key_set, &arts, SMOKE_CHAIN_ID).unwrap();
    (key_set, state)
}

// Rebuild a minimal EvmTestingContext from the produced PredeployState so
// integration assertions can invoke staking getters (`getEpochCommittee`,
// `getConsensusKeys`) against the same EVM state the runtime nodes will
// see at genesis.
fn ctx_from_predeploy(state: &PredeployState) -> EvmTestingContext {
    let fluent_contracts: Vec<_> = fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS
        .values()
        .cloned()
        .collect();
    let mut ctx = EvmTestingContext::default().with_contracts(&fluent_contracts);
    // The predeploy snapshot stores production-form bytecode — EVM runtime code
    // wrapped as OwnableAccount(PRECOMPILE_EVM_RUNTIME, ..) with the 0xEF44 magic
    // (see bootstrap::snapshot). Executing it requires the rWASM path (which
    // delegates the wrapper to the EVM_RUNTIME precompile), exactly like the
    // production node; the mainnet-legacy path would read 0xEF as an invalid
    // opcode and halt OpcodeNotFound.
    ctx.disabled_rwasm = false;
    ctx.cfg.limit_contract_code_size = Some(usize::MAX);
    ctx.cfg.limit_contract_initcode_size = Some(usize::MAX);
    ctx.cfg.disable_eip3607 = true;
    ctx.cfg.chain_id = SMOKE_CHAIN_ID;
    ctx.cfg.tx_chain_id_check = false;

    for (addr, code) in &state.bytecode_by_address {
        ctx.add_bytecode(*addr, code.clone());
    }
    for (addr, balance) in &state.balance_by_address {
        ctx.add_balance(*addr, *balance);
    }
    for (addr, storage) in &state.storage_by_address {
        for (slot, value) in storage {
            ctx.db
                .insert_account_storage(*addr, U256::from_be_bytes(slot.0), U256::from_be_bytes(value.0))
                .unwrap();
        }
    }
    ctx
}

sol! {
    struct ConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }
    interface IStakingView {
        function getEpochCommittee(uint64 epoch) external view returns (address[] memory);
        function getConsensusKeys(address validator) external view returns (ConsensusKeys memory);
    }
}

fn eth_call(ctx: &mut EvmTestingContext, to: Address, input: Bytes) -> Vec<u8> {
    let caller = Address::from([0xaa; 20]);
    ctx.add_balance(caller, U256::from(10u128).pow(U256::from(20)));
    let res = ctx.call_evm_tx(caller, to, input, Some(50_000_000), None);
    assert!(res.is_success(), "view call reverted: {res:?}");
    res.output().cloned().unwrap_or_default().to_vec()
}

#[test]
fn bootstrap_produces_all_predeploy_bytecode() {
    let (_keys, state) = run_bootstrap(2);

    for addr in [
        STAKING_ADDR,
        CHAIN_CONFIG_ADDR,
        STAKING_POOL_ADDR,
        SYSTEM_REWARD_ADDR,
        GOVERNANCE_ADDR,
        LIVENESS_SLASHING_ADDR,
        STAKING_TOKEN_ADDR,
        BLS_VERIFIER_ADDR,
    ] {
        let code = state
            .bytecode_by_address
            .get(&addr)
            .unwrap_or_else(|| panic!("no bytecode at canonical address {addr:?}"));
        assert!(!code.is_empty(), "empty bytecode at {addr:?}");
    }
}

#[test]
fn bootstrap_commits_epoch_zero_committee() {
    let (key_set, state) = run_bootstrap(2);
    let mut ctx = ctx_from_predeploy(&state);

    let calldata = IStakingView::getEpochCommitteeCall { epoch: 0 }.abi_encode();
    let out = eth_call(&mut ctx, STAKING_ADDR, calldata.into());
    let committee = <Vec<Address>>::abi_decode(&out).expect("decode getEpochCommittee result");

    assert_eq!(committee.len(), key_set.validators.len());
    let expected: Vec<Address> = key_set.validators.iter().map(|v| v.l2_signer.address()).collect();
    for addr in &expected {
        assert!(committee.contains(addr), "validator {addr:?} missing from committee");
    }
}

#[test]
fn bootstrap_registers_consensus_keys_per_validator() {
    let (key_set, state) = run_bootstrap(2);
    let mut ctx = ctx_from_predeploy(&state);

    for v in &key_set.validators {
        let addr = v.l2_signer.address();
        let calldata = IStakingView::getConsensusKeysCall { validator: addr }.abi_encode();
        let out = eth_call(&mut ctx, STAKING_ADDR, calldata.into());

        let keys = ConsensusKeys::abi_decode(&out).expect("decode getConsensusKeys");
        assert!(!keys.blsPubkey.is_empty(), "BLS pubkey empty for {addr:?}");
        assert_ne!(keys.peerPubkey, B256::ZERO, "peer pubkey zero for {addr:?}");
    }
}
