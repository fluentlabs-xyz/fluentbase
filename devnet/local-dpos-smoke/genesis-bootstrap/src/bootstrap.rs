use alloy_primitives::{address, Address, Bytes, B256, U256};
use alloy_sol_types::{SolCall, SolValue};
use eyre::WrapErr;
use fluentbase_testing::EvmTestingContext;
use std::collections::HashMap;

use crate::artifacts::{Artefacts, ContractArtefact};
use crate::keys::KeySet;
use crate::pop;

// Canonical predeploy addresses. Chosen visually-distinctive (0x...520N
// echoes chain_id=2026's older 0x5201 devnet pattern) and well clear of
// EIP-2537/2935/etc precompiles at 0x01..0x12. Storage layout under each
// of these addresses is identical to what the (matching impl's) UUPS proxy
// would have, because we deploy impl-direct.
pub const STAKING_ADDR: Address = address!("0x0000000000000000000000000000000000005201");
pub const CHAIN_CONFIG_ADDR: Address = address!("0x0000000000000000000000000000000000005202");
pub const STAKING_POOL_ADDR: Address = address!("0x0000000000000000000000000000000000005203");
pub const SYSTEM_REWARD_ADDR: Address = address!("0x0000000000000000000000000000000000005204");
pub const GOVERNANCE_ADDR: Address = address!("0x0000000000000000000000000000000000005205");
// LivenessSlashing is NOT a configurable predeploy: the block executor
// system-calls it at the fixed genesis address
// `fluentbase_types::PRECOMPILE_LIVENESS_SLASHING` (evm.rs
// `apply_pre_execution_changes` → processBitmap), unlike the staking/chain-config
// predeploys above whose addresses reach the executor via `--dpos.staking-config`.
// It MUST be deployed at that exact address — deploying it at a 0x...520N scheme
// address silently no-ops processBitmap (a system call to an empty address
// returns EVM `Success` with no state change, so `lastProcessedBlock`/`missCount`
// never move). Bound to the constant so the two can never drift.
pub const LIVENESS_SLASHING_ADDR: Address = fluentbase_types::PRECOMPILE_LIVENESS_SLASHING;
pub const STAKING_TOKEN_ADDR: Address = address!("0x0000000000000000000000000000000000005207");
// Non-upgradeable; no initialize/proxy. Wired into ChainConfig via
// setBlsVerifier (`onlyFromGovernance`) — bootstrap spoofs caller =
// GOVERNANCE_ADDR (immutable target of the modifier).
pub const BLS_VERIFIER_ADDR: Address = address!("0x0000000000000000000000000000000000005208");
// DELEGATECALL'd library that `Staking` is linked against (forge extracted the
// DPoS logic into it). Deployed at this fixed address; `artifacts::load` links
// `Staking`'s `__$StakingDpos$__` placeholders to it. Stateless library — no
// constructor args, no storage.
pub const STAKING_DPOS_ADDR: Address = address!("0x0000000000000000000000000000000000005209");
// Second DELEGATECALL'd library `Staking` is linked against — forge extracted the
// staking-economics logic (delegation / rewards / fees / claims) into it to keep
// `Staking` runtime bytecode under EIP-170. Deployed at this fixed address;
// `artifacts::load` links `Staking`'s `__$StakingEconomics$__` placeholders to it.
// Stateless library — no constructor args, no storage.
pub const STAKING_ECONOMICS_ADDR: Address = address!("0x000000000000000000000000000000000000520a");
// Stateless equivocation-evidence decoder (no storage / constructor, like the BLS
// verifier). Wired into ChainConfig via setEvidenceDecoder (`onlyFromGovernance`);
// without it `Staking._slashEquivocation` reverts EvidenceDecoderNotConfigured, so
// the byzantine equivocation smoke's slash can never land.
pub const EVIDENCE_DECODER_ADDR: Address = address!("0x000000000000000000000000000000000000520b");

// EVM canonical SYSTEM_CALLER per StakingContext.sol:17 — used to satisfy
// the `onlySystemCall` modifier on `commitEpochCommittee`.
const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

#[derive(Debug, Default)]
pub struct PredeployState {
    pub bytecode_by_address: HashMap<Address, Bytes>,
    pub storage_by_address: HashMap<Address, HashMap<B256, B256>>,
    pub balance_by_address: HashMap<Address, U256>,
}

// Each contract's `initialize(...)` lives inside its own interface block so
// the function name in the `sol!` source is `initialize` — alloy's selector
// is `keccak256(name + sig)[:4]`, so renaming the function (e.g. to
// `stakingInitialize`) silently produces a wrong selector and the contract
// dispatcher reverts with empty output. Interface namespacing keeps the
// on-chain name `initialize` while giving each contract its own Rust type
// (e.g. `IStaking::initializeCall`).
mod abi {
    use alloy_sol_types::sol;
    sol! {
        interface IChainConfig {
            function initialize(
                address initialOwner,
                uint32 activeValidatorsLength,
                uint32 epochBlockInterval,
                uint32 misdemeanorThreshold,
                uint32 felonyThreshold,
                uint32 validatorJailEpochLength,
                uint32 undelegatePeriod,
                uint256 minValidatorStakeAmount,
                uint256 minStakingAmount,
                uint64 dposActivationBlock
            ) external;
        }
        interface IStaking {
            function initialize(
                address initialOwner,
                address[] validators,
                uint256[] initialStakes,
                uint16 commissionRate
            ) external;
            function setConsensusKeys(
                address validatorAddress,
                bytes blsPubkeyUncompressed,
                bytes blsPoPUncompressed,
                bytes32 peerPubkey
            ) external;
            function commitEpochCommittee(address[] committee) external;
        }
        interface IStakingPool {
            function initialize(address initialOwner) external;
        }
        interface ILivenessSlashing {
            function initialize(address initialOwner) external;
        }
        interface ISystemReward {
            function initialize(
                address initialOwner,
                address[] accounts,
                uint16[] shares
            ) external;
        }
        interface IFluentGovernance {
            function initialize(address initialOwner, uint32 initialVotingPeriod) external;
        }
        interface IERC20 {
            function approve(address spender, uint256 value) external returns (bool);
        }
        interface IChainConfigGovernance {
            function setBlsVerifier(address newValue) external;
            function setEvidenceDecoder(address newValue) external;
        }
    }
}

pub fn run(keys: &KeySet, artefacts: &Artefacts, chain_id: u64) -> eyre::Result<PredeployState> {
    // PRECOMPILE_EVM_RUNTIME needs to be registered before any plain
    // EVM (`deployedBytecode`) deploy through `deploy_evm_tx` — without
    // it the EVM aborts with `MalformedBuiltinParams`. Mirrors the
    // e2e/src/lib.rs `with_full_genesis` trait impl.
    let fluent_contracts: Vec<_> = fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS
        .values()
        .cloned()
        .collect();
    let mut ctx = EvmTestingContext::default().with_contracts(&fluent_contracts);
    // Use mainnet revm path (not rWASM); we deploy plain EVM bytecode
    // and don't need fluentbase's WASM runtime. e2e/benches use the same
    // setting (e2e/benches/greeting.rs:15, e2e/src/nitro.rs:37).
    ctx.disabled_rwasm = true;
    // Staking.sol runtime bytecode exceeds EIP-170's 24 KB limit (it
    // packages slashing + BLS verifier + committee bookkeeping). Prod
    // deploys via UUPS proxy so the impl is on a side address whose
    // immutables don't have to be re-pointed; we deploy impl-direct,
    // so disable the cap (initcode and
    // runtime) for the in-process bootstrap session.
    ctx.cfg.limit_contract_code_size = Some(usize::MAX);
    ctx.cfg.limit_contract_initcode_size = Some(usize::MAX);
    // EIP-3607 (RejectCallerWithCode) blocks tx where caller already has
    // code. We need to spoof caller = GOVERNANCE_ADDR (a deployed
    // contract) to satisfy ChainConfig's `onlyFromGovernance` modifier
    // (checks `msg.sender == _governanceContract`, no code-shape check).
    // Same applies to SYSTEM_CALLER for `onlySystemCall`. Disable 3607
    // for the in-process bootstrap session only — not a real chain.
    ctx.cfg.disable_eip3607 = true;
    // block.chainid drives Staking._fluentNamespace() (= "FLUENT_DPOS_V1_"
    // ‖ u64 BE chain_id), which the contract uses both as the PoP-signed
    // message and the slashing namespace. The Rust-side PoP is signed
    // with `fluent_namespace(chain_id)` — both MUST agree, else
    // verifier.verify returns false → InvalidProofOfPossession.
    ctx.cfg.chain_id = chain_id;
    // TxBuilder::create / TxBuilder::call leave tx.chain_id at its
    // TxEnv::default() value of Some(1), which then disagrees with our
    // cfg.chain_id = 2026 and trips the EIP-155 chain-ID check. We don't
    // care about replay protection in an in-process bootstrap session,
    // so disable the check entirely instead of patching each TxEnv.
    ctx.cfg.tx_chain_id_check = false;
    let deployer = keys.governance_signer.address();
    ctx.add_balance(deployer, U256::from(10u128).pow(U256::from(22)));

    let context_args = (
        STAKING_ADDR,
        SYSTEM_REWARD_ADDR,
        STAKING_POOL_ADDR,
        GOVERNANCE_ADDR,
        CHAIN_CONFIG_ADDR,
        STAKING_TOKEN_ADDR,
    );
    let context_args_encoded = context_args.abi_encode_sequence();

    let staking_constructor = (
        STAKING_ADDR,
        SYSTEM_REWARD_ADDR,
        STAKING_POOL_ADDR,
        GOVERNANCE_ADDR,
        CHAIN_CONFIG_ADDR,
        STAKING_TOKEN_ADDR,
        LIVENESS_SLASHING_ADDR,
    )
        .abi_encode_sequence();

    // ChainConfig takes the 6 StakingContext addresses PLUS the F1 immutable
    // `minUndelegateBlocks` (devnet = 0, guard off). Encoded separately from the
    // shared `context_args_encoded`, which the other 6-arg UUPS impls reuse.
    let chain_config_constructor = (
        STAKING_ADDR,
        SYSTEM_REWARD_ADDR,
        STAKING_POOL_ADDR,
        GOVERNANCE_ADDR,
        CHAIN_CONFIG_ADDR,
        STAKING_TOKEN_ADDR,
        U256::ZERO, // minUndelegateBlocks: F1 floor off on devnet
    )
        .abi_encode_sequence();

    let governance_constructor = (STAKING_ADDR, CHAIN_CONFIG_ADDR).abi_encode_sequence();

    // MockBlendToken: constructor mints to deployer → storage MUST be
    // copied (balanceOf, totalSupply). The 6 UUPS impls below set ONLY
    // immutables + call `_disableInitializers()`, which writes to the
    // OZ initialized slot (`Initializable.STORAGE_LOCATION`); copying
    // that to canonical would make `initialize()` revert with
    // `InvalidInitialization()`. So: copy storage for MockBlendToken,
    // skip storage for UUPS impls.
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.mock_blend_token,
        STAKING_TOKEN_ADDR,
        &[],
        true,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.system_reward,
        SYSTEM_REWARD_ADDR,
        &context_args_encoded,
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.staking_pool,
        STAKING_POOL_ADDR,
        &context_args_encoded,
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.chain_config,
        CHAIN_CONFIG_ADDR,
        &chain_config_constructor,
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.liveness_slashing,
        LIVENESS_SLASHING_ADDR,
        &context_args_encoded,
        false,
    )?;
    // Deploy the DELEGATECALL'd libraries FIRST — `Staking`'s bytecode is linked
    // against `STAKING_DPOS_ADDR` + `STAKING_ECONOMICS_ADDR` (see `artifacts::load`).
    // Stateless libraries: no constructor args, no storage copy.
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.staking_dpos,
        STAKING_DPOS_ADDR,
        &[],
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.staking_economics,
        STAKING_ECONOMICS_ADDR,
        &[],
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.staking,
        STAKING_ADDR,
        &staking_constructor,
        false,
    )?;
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.governance,
        GOVERNANCE_ADDR,
        &governance_constructor,
        false,
    )?;
    // BLS12381Verifier is stateless (no storage, no constructor args) —
    // just place the runtime bytecode at the canonical address. We still
    // route through deploy_to_canonical to keep one code path for all
    // predeploys.
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.bls_verifier,
        BLS_VERIFIER_ADDR,
        &[],
        false,
    )?;
    // SimplexEvidenceDecoder is likewise stateless (no storage / constructor) —
    // place its runtime bytecode at the canonical address through the same path.
    deploy_to_canonical(
        &mut ctx,
        deployer,
        &artefacts.evidence_decoder,
        EVIDENCE_DECODER_ADDR,
        &[],
        false,
    )?;

    // ChainConfig enforces non-zero minStake values (revert
    // "minValidatorStakeAmount") and Staking._addValidator enforces
    // `initialStake >= minValidatorStakeAmount` (line 731). Stakes must
    // also be `% BALANCE_COMPACT_PRECISION == 0` where the precision
    // is `1e10` (Staking.sol:78). Pick 1 BLEND (1e18) as min stake +
    // initial — multiple of 1e10, smoke uses no real economics.
    let smoke_min_stake = U256::from(10u128).pow(U256::from(18));
    let chain_config_init = abi::IChainConfig::initializeCall {
        initialOwner: deployer,
        activeValidatorsLength: keys.validators.len() as u32,
        epochBlockInterval: 32,
        misdemeanorThreshold: 100,
        felonyThreshold: 1000,
        validatorJailEpochLength: 16,
        undelegatePeriod: 16,
        minValidatorStakeAmount: smoke_min_stake,
        minStakingAmount: smoke_min_stake,
        // Aligned (% epochBlockInterval=32) future block at which DPoS epoch
        // numbering rebases to 0 — the sequencer→DPoS migration anchor lands in
        // relative epoch 0 ([64, 95]). `_migrate_to_dpos` (scripts/lib.sh) waits
        // for the sequencer to finalize >= this block before swapping; the two MUST
        // agree (DPOS_ACTIVATION_BLOCK there). `_currentEpoch` clamps to 0 for
        // the pre-activation sequencer-era window. Absolute epoch here is 64/32 = 2, so
        // the smoke exercises the epoch>=1 cold-start (the bug-#2 scenario).
        dposActivationBlock: 64,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        CHAIN_CONFIG_ADDR,
        chain_config_init.into(),
        "ChainConfig.initialize",
    )?;

    let initial_stakes = vec![smoke_min_stake; keys.validators.len()];
    let total_stake = smoke_min_stake * U256::from(keys.validators.len() as u64);
    let validator_addrs: Vec<Address> = keys
        .validators
        .iter()
        .map(|v| v.l2_signer.address())
        .collect();

    // Staking._addValidator pulls each validator's initial stake via
    // BLEND.transferFrom(initialOwner, staking, stake). MockBlendToken's
    // constructor mints the entire supply to `msg.sender` (= deployer),
    // so the funds are present, but we still need to grant allowance to
    // STAKING_ADDR before calling Staking.initialize, otherwise the call
    // reverts with ERC20InsufficientAllowance.
    let approve_call = abi::IERC20::approveCall {
        spender: STAKING_ADDR,
        value: total_stake,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        STAKING_TOKEN_ADDR,
        approve_call.into(),
        "BLEND.approve(staking)",
    )?;

    let staking_init = abi::IStaking::initializeCall {
        initialOwner: deployer,
        validators: validator_addrs.clone(),
        initialStakes: initial_stakes,
        commissionRate: 0,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        STAKING_ADDR,
        staking_init.into(),
        "Staking.initialize",
    )?;

    let pool_init = abi::IStakingPool::initializeCall {
        initialOwner: deployer,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        STAKING_POOL_ADDR,
        pool_init.into(),
        "StakingPool.initialize",
    )?;

    let liveness_init = abi::ILivenessSlashing::initializeCall {
        initialOwner: deployer,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        LIVENESS_SLASHING_ADDR,
        liveness_init.into(),
        "LivenessSlashing.initialize",
    )?;

    // SystemReward._updateDistributionShare requires sum(shares) ==
    // SHARE_MAX_VALUE (10_000 bps). Smoke has no real fee economics, so
    // route 100% of system reward to the deployer EOA — a single-entry
    // distribution table satisfies the require + leaves changes for
    // later via the governance-owned update path.
    let sys_reward_init = abi::ISystemReward::initializeCall {
        initialOwner: deployer,
        accounts: vec![deployer],
        shares: vec![10_000],
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        SYSTEM_REWARD_ADDR,
        sys_reward_init.into(),
        "SystemReward.initialize",
    )?;

    let gov_init = abi::IFluentGovernance::initializeCall {
        initialOwner: deployer,
        initialVotingPeriod: 1,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        GOVERNANCE_ADDR,
        gov_init.into(),
        "FluentGovernance.initialize",
    )?;

    // Wire BLS verifier through ChainConfig BEFORE registering keys —
    // setConsensusKeys reads chainConfig.getBlsVerifier() and reverts
    // BlsVerifierNotConfigured if it's address(0). setBlsVerifier is
    // `onlyFromGovernance`, where governance is the FluentGovernance
    // immutable set in ChainConfig's constructor (GOVERNANCE_ADDR), so
    // we spoof caller = GOVERNANCE_ADDR — the modifier checks
    // `msg.sender == _governanceContract` only.
    let set_verifier = abi::IChainConfigGovernance::setBlsVerifierCall {
        newValue: BLS_VERIFIER_ADDR,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        GOVERNANCE_ADDR,
        CHAIN_CONFIG_ADDR,
        set_verifier.into(),
        "ChainConfig.setBlsVerifier",
    )?;

    // Wire the equivocation-evidence decoder through ChainConfig (same
    // `onlyFromGovernance` spoof). Without it `Staking._slashEquivocation` reverts
    // `EvidenceDecoderNotConfigured` — the byzantine equivocation smoke's on-chain
    // slash would never land (honest peers detect+block the equivocator, but the
    // jail never happens). setConsensusKeys does not need it, so historically it was
    // left unset; the equivocation smoke is the first path that exercises slashing.
    let set_decoder = abi::IChainConfigGovernance::setEvidenceDecoderCall {
        newValue: EVIDENCE_DECODER_ADDR,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        GOVERNANCE_ADDR,
        CHAIN_CONFIG_ADDR,
        set_decoder.into(),
        "ChainConfig.setEvidenceDecoder",
    )?;

    register_validators(&mut ctx, keys)?;
    commit_initial_committee(&mut ctx, keys)?;
    // No genesis beacon key is committed: the beacon is always-on live DKG and is
    // consumed internally (the per-block seed rides the consensus cert; there is no
    // on-chain PK_E — that layer was removed, DPOS_ARCHITECTURE §8.11).

    Ok(snapshot(
        &mut ctx,
        &[
            STAKING_ADDR,
            STAKING_DPOS_ADDR,
            STAKING_ECONOMICS_ADDR,
            CHAIN_CONFIG_ADDR,
            STAKING_POOL_ADDR,
            SYSTEM_REWARD_ADDR,
            GOVERNANCE_ADDR,
            LIVENESS_SLASHING_ADDR,
            STAKING_TOKEN_ADDR,
            BLS_VERIFIER_ADDR,
            EVIDENCE_DECODER_ADDR,
        ],
    ))
}

fn deploy_to_canonical(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    artefact: &ContractArtefact,
    canonical: Address,
    constructor_args: &[u8],
    copy_storage: bool,
) -> eyre::Result<()> {
    let mut init = artefact.init_bytecode.to_vec();
    init.extend_from_slice(constructor_args);
    let create_addr = ctx
        .deploy_evm_tx_result(deployer, init.into())
        .map_err(|res| eyre::eyre!("deploy to {canonical:?} reverted: {res:?}"))?;

    let src = ctx
        .db
        .cache
        .accounts
        .get(&create_addr)
        .cloned()
        .ok_or_else(|| eyre::eyre!("freshly-deployed account {create_addr:?} not found in db"))?;
    let runtime_code = src
        .info
        .code
        .as_ref()
        .ok_or_else(|| eyre::eyre!("deployed account has no code"))?
        .original_bytes();
    ctx.add_bytecode(canonical, runtime_code);
    ctx.add_balance(canonical, src.info.balance);
    if copy_storage {
        for (slot, value) in &src.storage {
            ctx.db
                .insert_account_storage(canonical, *slot, *value)
                .wrap_err("insert storage slot on canonical")?;
        }
    }
    tracing::debug!(
        contract = ?canonical,
        create_addr = ?create_addr,
        storage_slots = if copy_storage { src.storage.len() } else { 0 },
        "deployed + copied to canonical"
    );
    Ok(())
}

fn call_or_die(
    ctx: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Bytes,
    label: &str,
) -> eyre::Result<()> {
    let res = ctx.call_evm_tx(caller, callee, input, Some(50_000_000), None);
    if !res.is_success() {
        return Err(eyre::eyre!(
            "{label} (caller={caller:?} → {callee:?}) reverted: {res:?}"
        ));
    }
    Ok(())
}

fn register_validators(ctx: &mut EvmTestingContext, keys: &KeySet) -> eyre::Result<()> {
    for v in &keys.validators {
        let p = pop::produce(&v.bls, keys.chain_id)?;
        let mut peer_bytes32 = [0u8; 32];
        use commonware_codec::Encode as _;
        let pk = commonware_cryptography::Signer::public_key(&v.peer).encode();
        peer_bytes32.copy_from_slice(pk.as_ref());

        let input = abi::IStaking::setConsensusKeysCall {
            validatorAddress: v.l2_signer.address(),
            blsPubkeyUncompressed: Bytes::copy_from_slice(&p.bls_pubkey_uncompressed),
            blsPoPUncompressed: Bytes::copy_from_slice(&p.bls_pop_uncompressed),
            peerPubkey: peer_bytes32.into(),
        }
        .abi_encode();

        // setConsensusKeys requires msg.sender == ownerAddress (Staking.sol:1095).
        // initialize() set ownerAddress = validator.l2_signer.address() for each
        // initial validator (Staking.sol:221 _addValidator(addr, addr, ...)).
        call_or_die(
            ctx,
            v.l2_signer.address(),
            STAKING_ADDR,
            input.into(),
            &format!("setConsensusKeys[validator-{}]", v.idx),
        )?;
    }
    Ok(())
}

fn commit_initial_committee(ctx: &mut EvmTestingContext, keys: &KeySet) -> eyre::Result<()> {
    // Canonical ed25519 ascending-peerPubkey order (G5 invariant —
    // crates/p2p/src/lib.rs:213, commonware_utils::ordered::Set ordering).
    use commonware_codec::Encode as _;
    let mut sorted: Vec<(Vec<u8>, Address)> = keys
        .validators
        .iter()
        .map(|v| {
            let pk = commonware_cryptography::Signer::public_key(&v.peer).encode();
            (pk.as_ref().to_vec(), v.l2_signer.address())
        })
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let committee: Vec<Address> = sorted.into_iter().map(|(_, addr)| addr).collect();

    let input = abi::IStaking::commitEpochCommitteeCall { committee }.abi_encode();
    // commitEpochCommittee is `onlySystemCall` (Staking.sol:1191 +
    // StakingContext.sol:63-64) — tx.caller must == SYSTEM_CALLER, NOT
    // block.coinbase.
    call_or_die(
        ctx,
        SYSTEM_CALLER,
        STAKING_ADDR,
        input.into(),
        "commitEpochCommittee[epoch=0]",
    )
}

fn snapshot(ctx: &mut EvmTestingContext, addrs: &[Address]) -> PredeployState {
    let mut bytecode_by_address = HashMap::new();
    let mut storage_by_address = HashMap::new();
    let mut balance_by_address = HashMap::new();

    for addr in addrs {
        let Some(account) = ctx.db.cache.accounts.get(addr) else {
            continue;
        };

        if let Some(bytecode) = &account.info.code {
            // Wrap deployed EVM runtime bytecode in OwnableAccount(EVM_RUNTIME, code).
            // Bootstrap runs with `disabled_rwasm = true` (plain revm), so deployed
            // contracts come out as plain LegacyAnalyzed EVM bytecode (0x60…).
            // Fluent's production executor (`execute_rwasm_frame` at
            // crates/revm/src/executor.rs:213,389) expects every contract to be
            // either rWASM-native or an OwnableAccount that delegates to a runtime
            // precompile — otherwise it returns `NotSupportedBytecode`, payload
            // build fails, no blocks. During normal CREATE through fluent's
            // rWASM EVM (crates/revm/src/evm.rs:362), revm wraps the new contract
            // as `OwnableAccount(PRECOMPILE_EVM_RUNTIME, runtime_bytecode)`; we
            // replicate that wrapping here so the genesis JSON's `code` field
            // starts with the 0xEF44 magic and `Bytecode::new_raw_checked` in the
            // running node round-trips back to OwnableAccount.
            let raw = bytecode.original_bytes();
            let wrapped: Bytes = if raw.starts_with(&[0xEF, 0x44]) || raw.starts_with(&[0xEF, 0x52])
            {
                raw
            } else {
                // EVM_RUNTIME reads metadata as `EthereumMetadata` (see
                // crates/evm/src/metadata.rs:23) — `[code_hash 32 bytes] || [bytecode]`
                // for legacy. Without the 32-byte hash prefix EVM_RUNTIME
                // interprets the first 32 bytes of EVM bytecode as the hash,
                // skips them, runs the rest, halts with StackOverflow.
                let metadata = fluentbase_evm::EthereumMetadata::new_legacy(raw).write_to_bytes();
                let mut buf = Vec::with_capacity(23 + metadata.len());
                buf.extend_from_slice(&[0xEF, 0x44, 0x00]);
                buf.extend_from_slice(fluentbase_types::PRECOMPILE_EVM_RUNTIME.as_slice());
                buf.extend_from_slice(&metadata);
                buf.into()
            };
            bytecode_by_address.insert(*addr, wrapped);
        }
        balance_by_address.insert(*addr, account.info.balance);

        let storage: HashMap<B256, B256> = account
            .storage
            .iter()
            .map(|(k, v)| (B256::from(*k), B256::from(*v)))
            .collect();
        if !storage.is_empty() {
            storage_by_address.insert(*addr, storage);
        }
    }

    PredeployState {
        bytecode_by_address,
        storage_by_address,
        balance_by_address,
    }
}
