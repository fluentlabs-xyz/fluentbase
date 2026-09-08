use std::path::PathBuf;

use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall, SolValue};
use fluentbase_genesis_bootstrap::{
    artifacts, bootstrap,
    bootstrap::{
        PredeployState, GOVERNANCE_ADDR, GOVERNANCE_VOTING_PERIOD_BLOCKS,
        STAKING_ADDR, STAKING_POOL_ADDR, STAKING_TOKEN_ADDR,
    },
    keys,
};
use fluentbase_testing::EvmTestingContext;

const SMOKE_CHAIN_ID: u64 = 2026;
const SMOKE_MNEMONIC: &str = "test test test test test test test test test test test junk";
/// The contract's `MIN_COMMITTEE_LENGTH` (`consts.rs:388`): `commitEpochCommittee`
/// reverts `ERR_COMMITTEE_TOO_SMALL` below it, so a bootstrap with fewer peers
/// cannot complete at all.
const SMOKE_PEERS: u32 = 4;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("contracts")
}

fn run_bootstrap() -> (keys::KeySet, PredeployState) {
    run_bootstrap_with(SMOKE_PEERS, bootstrap::BootstrapParams::default())
}

fn run_bootstrap_with(
    peers: u32,
    params: bootstrap::BootstrapParams,
) -> (keys::KeySet, PredeployState) {
    let key_set = keys::derive(SMOKE_MNEMONIC, peers, SMOKE_CHAIN_ID).unwrap();
    let arts = artifacts::load(&contracts_dir()).unwrap();
    let state = bootstrap::run(&key_set, &arts, SMOKE_CHAIN_ID, &params).unwrap();
    (key_set, state)
}

// Rebuild a minimal EvmTestingContext from the produced PredeployState so
// integration assertions can invoke staking getters against the same EVM state
// the runtime nodes will see at genesis.
fn ctx_from_predeploy(state: &PredeployState) -> EvmTestingContext {
    let fluent_contracts: Vec<_> = fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS
        .values()
        .cloned()
        .collect();
    let mut ctx = EvmTestingContext::default().with_contracts(&fluent_contracts);
    // The predeploy snapshot stores production-form bytecode — the rWasm staking
    // module verbatim, and EVM runtime code wrapped as
    // OwnableAccount(PRECOMPILE_EVM_RUNTIME, ..) with the 0xEF44 magic (see
    // bootstrap::wrap_for_rwasm). Executing either requires the rWASM path, exactly
    // like the production node; the mainnet-legacy path would read 0xEF as an invalid
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
                .insert_account_storage(
                    *addr,
                    U256::from_be_bytes(slot.0),
                    U256::from_be_bytes(value.0),
                )
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
        function getEpochCommitteeWithStakes(uint64 epoch) external view returns (
            address[] addrs, ConsensusKeys[] keys, uint256[] stakes, bool[] tombstoned);
        function getConsensusKeys(address validator) external view returns (ConsensusKeys memory);
        function getProductionLivenessDisabled() external view returns (bool);
        function getBlendStipendPerEpoch() external view returns (uint256);
        function getDposActivationBlock() external view returns (uint64);
        function getActiveValidatorsLength() external view returns (uint32);
    }
    interface IERC20View {
        function balanceOf(address account) external view returns (uint256);
    }
    interface IGovernorView {
        function votingPeriod() external view returns (uint256);
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
    let (_keys, state) = run_bootstrap();

    for addr in [
        STAKING_ADDR,
        STAKING_POOL_ADDR,
        GOVERNANCE_ADDR,
        STAKING_TOKEN_ADDR,
    ] {
        let code = state
            .bytecode_by_address
            .get(&addr)
            .unwrap_or_else(|| panic!("no bytecode at canonical address {addr:?}"));
        assert!(!code.is_empty(), "empty bytecode at {addr:?}");
    }
}

/// The two configuration decisions `initialize` deliberately does NOT make. It
/// writes `productionLivenessDisabled = true` (the tier ships off, and an unwritten
/// slot would ship it on) and leaves the stipend at its zero kill-switch, so both
/// have to arrive as governance calls. Miss either and the devnet runs with the
/// liveness tier dark and every validator reward a permanent no-op — silently, since
/// both states are legal. Reads the genesis state through the production rWASM path.
#[test]
fn bootstrap_makes_the_two_config_calls_initialize_cannot() {
    let (_keys, state) = run_bootstrap();
    let mut ctx = ctx_from_predeploy(&state);

    let disabled_out = eth_call(
        &mut ctx,
        STAKING_ADDR,
        IStakingView::getProductionLivenessDisabledCall {}
            .abi_encode()
            .into(),
    );
    let disabled = bool::abi_decode(&disabled_out).expect("decode getProductionLivenessDisabled");
    assert!(!disabled, "production-liveness tier must be ON in devnet");

    let stipend_out = eth_call(
        &mut ctx,
        STAKING_ADDR,
        IStakingView::getBlendStipendPerEpochCall {}
            .abi_encode()
            .into(),
    );
    let stipend = U256::abi_decode(&stipend_out).expect("decode getBlendStipendPerEpoch");
    assert!(
        stipend > U256::ZERO,
        "per-epoch BLEND stipend must be ON in devnet"
    );
}

#[test]
fn bootstrap_commits_epoch_zero_committee() {
    let (key_set, state) = run_bootstrap();
    let mut ctx = ctx_from_predeploy(&state);

    let calldata = IStakingView::getEpochCommitteeCall { epoch: 0 }.abi_encode();
    let out = eth_call(&mut ctx, STAKING_ADDR, calldata.into());
    let committee = <Vec<Address>>::abi_decode(&out).expect("decode getEpochCommittee result");

    assert_eq!(committee.len(), key_set.validators.len());
    let expected: Vec<Address> = key_set
        .validators
        .iter()
        .map(|v| v.l2_signer.address())
        .collect();
    for addr in &expected {
        assert!(
            committee.contains(addr),
            "validator {addr:?} missing from committee"
        );
    }
}

/// Peer-key ascending IS the consensus index space: the node writes a commonware
/// `Participant` index into the block and the contract resolves the victim of a slash
/// positionally from this array. The contract produces the order and nothing on this
/// side re-derives it, so this is the only place the agreement is checked — and a
/// duplicate would make two positions indistinguishable, hence strict.
#[test]
fn committed_committee_is_strictly_ascending_on_peer_pubkey() {
    let (_keys, state) = run_bootstrap();
    let mut ctx = ctx_from_predeploy(&state);

    let calldata = IStakingView::getEpochCommitteeWithStakesCall { epoch: 0 }.abi_encode();
    let out = eth_call(&mut ctx, STAKING_ADDR, calldata.into());
    let decoded = IStakingView::getEpochCommitteeWithStakesCall::abi_decode_returns(&out)
        .expect("decode getEpochCommitteeWithStakes");

    assert_eq!(decoded.keys.len(), decoded.addrs.len());
    for pair in decoded.keys.windows(2) {
        assert!(
            pair[0].peerPubkey < pair[1].peerPubkey,
            "committee not strictly ascending on peer pubkey: {:?} then {:?}",
            pair[0].peerPubkey,
            pair[1].peerPubkey
        );
    }
}

/// The sim/soak shape: a derived identity POOL wider than the set of nodes actually
/// run, BLEND on `validator-0`'s owner key rather than on the governance signer, and
/// no activation scheduled at genesis. Each of the three defaults is wrong for that
/// stand in a way that is silent or fatal — a committee holding identities with no
/// node behind them never finalizes, a supply on an unspendable account fails every
/// later delegation, and an activation block already in the past cannot be moved.
#[test]
fn sim_shape_seats_a_prefix_and_defers_activation() {
    const POOL: u32 = 6;
    const SEATED: usize = 4;
    let pool_keys = keys::derive(SMOKE_MNEMONIC, POOL, SMOKE_CHAIN_ID).unwrap();
    let blend_holder = pool_keys.validators[0].l2_signer.address();
    let governance = pool_keys.governance_signer.address();
    let (key_set, state) = run_bootstrap_with(
        POOL,
        bootstrap::BootstrapParams {
            committee_size: Some(SEATED),
            blend_holder: Some(blend_holder),
            dpos_activation_block: Some(0),
        },
    );
    let mut ctx = ctx_from_predeploy(&state);

    let out = eth_call(
        &mut ctx,
        STAKING_ADDR,
        IStakingView::getEpochCommitteeCall { epoch: 0 }
            .abi_encode()
            .into(),
    );
    let committee = <Vec<Address>>::abi_decode(&out).expect("decode getEpochCommittee");
    let expected: Vec<Address> = key_set.validators[..SEATED]
        .iter()
        .map(|v| v.l2_signer.address())
        .collect();
    assert_eq!(committee.len(), SEATED);
    for addr in &expected {
        assert!(committee.contains(addr), "seated {addr:?} not in committee");
    }
    for v in &key_set.validators[SEATED..] {
        assert!(
            !committee.contains(&v.l2_signer.address()),
            "unseated validator-{} reached the committee",
            v.idx
        );
    }

    let cap_out = eth_call(
        &mut ctx,
        STAKING_ADDR,
        IStakingView::getActiveValidatorsLengthCall {}
            .abi_encode()
            .into(),
    );
    let cap = u32::abi_decode(&cap_out).expect("decode getActiveValidatorsLength");
    assert_eq!(cap as usize, SEATED);

    let activation_out = eth_call(
        &mut ctx,
        STAKING_ADDR,
        IStakingView::getDposActivationBlockCall {}
            .abi_encode()
            .into(),
    );
    let activation = u64::abi_decode(&activation_out).expect("decode getDposActivationBlock");
    assert_eq!(activation, 0, "activation must stay unscheduled at genesis");

    let holder_balance = balance_of(&mut ctx, blend_holder);
    assert!(
        holder_balance > U256::ZERO,
        "BLEND supply did not reach the named holder"
    );
    assert_eq!(
        balance_of(&mut ctx, governance),
        U256::ZERO,
        "governance signer still holds BLEND the harness cannot spend"
    );
}

/// `bare` installs the Governor and NOTHING else. Both halves are load-bearing: the
/// staking module compiles `GENESIS_GOVERNANCE` in as the sole caller its setters
/// accept, so a missing Governor makes every post-delivery governance write hit an
/// address with no code — which answers Success-with-empty-output, not a revert, so the
/// stand reads it as a proposal that never became Active rather than as a fault. And
/// the staking address must stay codeless, because that is what the stand's premise and
/// the reader's code-presence probe both key off.
#[test]
fn bare_installs_only_the_governor() {
    let key_set = keys::derive(SMOKE_MNEMONIC, SMOKE_PEERS, SMOKE_CHAIN_ID).unwrap();
    let governance_init = artifacts::load_governance(&contracts_dir()).unwrap();
    let state = bootstrap::run_governance_only(&key_set, &governance_init, SMOKE_CHAIN_ID).unwrap();

    let governor = state
        .bytecode_by_address
        .get(&GOVERNANCE_ADDR)
        .expect("no code at GENESIS_GOVERNANCE");
    assert!(!governor.is_empty());
    assert_eq!(
        state.bytecode_by_address.len(),
        1,
        "bare installed more than the Governor: {:?}",
        state.bytecode_by_address.keys().collect::<Vec<_>>()
    );
    for addr in [
        STAKING_ADDR,
        STAKING_POOL_ADDR,
        STAKING_TOKEN_ADDR,
    ] {
        assert!(
            !state.bytecode_by_address.contains_key(&addr),
            "bare must leave {addr:?} codeless"
        );
    }

    // Initialized, and executable in the production wrapped form — a Governor whose
    // `initialize` silently did nothing looks identical in the alloc.
    let mut ctx = ctx_from_predeploy(&state);
    let out = eth_call(
        &mut ctx,
        GOVERNANCE_ADDR,
        IGovernorView::votingPeriodCall {}.abi_encode().into(),
    );
    let period = U256::abi_decode(&out).expect("decode votingPeriod");
    assert_eq!(
        period,
        U256::from(GOVERNANCE_VOTING_PERIOD_BLOCKS),
        "bare's Governor must carry the same voting window as full's"
    );
}

fn balance_of(ctx: &mut EvmTestingContext, account: Address) -> U256 {
    let out = eth_call(
        ctx,
        STAKING_TOKEN_ADDR,
        IERC20View::balanceOfCall { account }.abi_encode().into(),
    );
    U256::abi_decode(&out).expect("decode balanceOf")
}

/// Every genesis validator is registered, and the identity the module stored is
/// the one `blst` compressed on this side.
///
/// The second half is a cross-implementation check that has no other home. The
/// module compresses the 256-byte EIP-2537 key itself now; EIP-2537 orders the
/// real coefficient of `x` first and zcash orders the imaginary one first, and
/// the y-sign is read from `y.c1` with `y.c0` only as a tiebreak. Get either
/// wrong and you get a well-formed 96-byte key that matches nothing any node
/// ever registered — which surfaces as slashing that silently never fires, not
/// as a failure here. `blst` produced the right-hand side; the arkworks
/// precompiles and the module's own byte handling produced the left.
#[test]
fn bootstrap_registers_each_validator_under_the_key_blst_compressed() {
    let (key_set, state) = run_bootstrap();
    let mut ctx = ctx_from_predeploy(&state);

    for v in &key_set.validators {
        let addr = v.l2_signer.address();
        let calldata = IStakingView::getConsensusKeysCall { validator: addr }.abi_encode();
        let out = eth_call(&mut ctx, STAKING_ADDR, calldata.into());

        let keys = ConsensusKeys::abi_decode(&out).expect("decode getConsensusKeys");
        assert_eq!(
            keys.blsPubkey.as_ref(),
            &v.bls.public_bytes()[..],
            "stored identity for {addr:?} is not the compressed key the node signs under"
        );
        assert_ne!(keys.peerPubkey, B256::ZERO, "peer pubkey zero for {addr:?}");
    }
}

/// A proof of possession made for a different chain does not get in.
///
/// This is the whole point of the change, exercised end to end: the module runs
/// the pairing itself, against precompiles at addresses fixed by the fork, and
/// there is no stored verifier address a governance call could point somewhere
/// friendlier. The keys below are real and their proofs are real — they are just
/// bound to `SMOKE_CHAIN_ID + 1`, and the namespace the module hashes under is
/// taken from `block.chainid`. Nothing about the encoding differs, so what
/// rejects this is the pairing and only the pairing.
#[test]
fn a_proof_of_possession_bound_to_another_chain_is_refused() {
    let foreign = keys::derive(SMOKE_MNEMONIC, SMOKE_PEERS, SMOKE_CHAIN_ID + 1).unwrap();
    let arts = artifacts::load(&contracts_dir()).unwrap();
    let outcome = bootstrap::run(
        &foreign,
        &arts,
        SMOKE_CHAIN_ID,
        &bootstrap::BootstrapParams::default(),
    );
    let error = outcome.expect_err("genesis accepted a proof of possession from another chain");
    let text = format!("{error:?}");
    assert!(
        text.contains("Staking.initialize"),
        "the refusal came from somewhere other than the genesis initialize: {text}"
    );

    // The same keys, under the chain they were signed for, go through — so what
    // the assertion above caught is the namespace and not some other difference
    // between the two runs.
    bootstrap::run(
        &foreign,
        &arts,
        SMOKE_CHAIN_ID + 1,
        &bootstrap::BootstrapParams::default(),
    )
    .expect("the same keys must bootstrap the chain they were signed for");
}
