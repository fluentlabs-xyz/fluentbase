use alloy_primitives::{address, Address, Bytes, B256, U256};
use alloy_sol_types::{SolCall, SolValue};
use eyre::WrapErr;
use fluentbase_testing::EvmTestingContext;
use std::collections::HashMap;

use crate::artifacts::Artefacts;
use crate::keys::KeySet;
use crate::pop;

// The staking system contract and the governance contract live at the two
// addresses `fluentbase_types` reserves for them. `GENESIS_GOVERNANCE` is
// COMPILED INTO the staking module as the sole caller its privileged setters
// accept (`util.rs::ensure_governance`), so the Governor deployed below must land
// at exactly that address or every governance write reverts `ERR_ONLY_GOVERNANCE`.
pub const STAKING_ADDR: Address = fluentbase_types::GENESIS_STAKING;
pub const GOVERNANCE_ADDR: Address = fluentbase_types::GENESIS_GOVERNANCE;
// The two predeploys that stayed Solidity keep their historical 0x...520N devnet
// slots (well clear of the EIP-2537/2935 precompiles at 0x01..0x12).
//
// `0x...5208` used to hold `BLS12381Verifier`. The staking module verifies BLS
// signatures itself now and reaches the EIP-2537 precompiles directly, so the
// slot is not filled and its address is not passed to anything. It is left
// vacant rather than reassigned, so a stale harness still pointing at it fails
// loudly instead of hitting somebody else's code.
pub const STAKING_POOL_ADDR: Address = address!("0x0000000000000000000000000000000000005203");
pub const STAKING_TOKEN_ADDR: Address = address!("0x0000000000000000000000000000000000005207");

// EVM canonical SYSTEM_CALLER (`consts.rs::SYSTEM_CALLER`) — used to satisfy the
// `ERR_ONLY_SYSTEM_CALL` guard on `commitEpochCommittee`.
const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

// `initialize` verifies one BLS proof-of-possession per genesis validator inside a
// single call (`initializer.rs` → `consensus::verify_consensus_keys`), each a
// hash-to-curve and a pairing the staking module now runs itself against the
// EIP-2537 precompiles, so the whole init sequence runs well past the 50 M an
// individual predeploy call used to need. This is an in-process bootstrap EVM with
// `BlockEnv::gas_limit = u64::MAX`, not a consensus budget.
const BOOTSTRAP_GAS_LIMIT: u64 = 500_000_000;

// Governor voting window, in blocks — the value
// `solidity-contracts/scripts/config/local-dpos-smoke/l2.json` gave the retired
// `DeployStaking` run for this same devnet. Not a tuning choice: the sim drives real
// governance rounds (setEpochBlockInterval, setDposActivationBlock, cap raises), and a
// 1-block window closes between the harness noticing a proposal is Active and its votes
// landing, so every proposal ends Defeated. The static smoke never exercised governance,
// which is why 1 survived until the sim moved off the runtime forge deploy onto this
// path. Shared by BOTH arms: the production-path stand governs a chain whose Governor
// came from `bare`, and a window that differed there would make that stand behave
// unlike every other one.
pub const GOVERNANCE_VOTING_PERIOD_BLOCKS: u32 = 10;

/// The three genesis facts that differ between the stands this binary serves.
///
/// Every field is `None` by default and every `None` resolves to the genesis-baked
/// smoke's behaviour, so a smoke invocation is byte-identical whether or not the
/// caller knows these exist. The sim/soak sets all three: it derives a large identity
/// POOL but runs containers for only the first few, it spends BLEND from
/// `validator-0`'s owner key, and it schedules DPoS activation itself through
/// governance once its bring-up phases are done.
#[derive(Debug, Default)]
pub struct BootstrapParams {
    /// Seat `validators[0..committee_size]` as the genesis committee and use that as
    /// the `activeValidatorsLength` cap. `None` ⇒ every derived identity, which is
    /// right only where `--peers` IS the committee.
    pub committee_size: Option<usize>,
    /// Account the whole MockBlendToken supply ends up on, and therefore the genesis
    /// stake sponsor and the `blendReserve` the stipend is drawn from. `None` ⇒ the
    /// governance signer, which is what the token's constructor mints to.
    pub blend_holder: Option<Address>,
    /// `dposActivationBlock`. `None` ⇒ `2 × epochBlockInterval`, which lands the
    /// migration anchor in absolute epoch 2. `Some(0)` is the contract's unscheduled
    /// sentinel — nothing is scheduled at genesis and governance sets the real value
    /// later.
    pub dpos_activation_block: Option<u64>,
}

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
        interface IStaking {
            function initialize(
                address initialStakeOwner,
                address[] validators,
                uint256[] initialStakes,
                bytes[] blsPubkeysUncompressed,
                bytes[] blsPopsUncompressed,
                bytes32[] peerPubkeys,
                uint16 commissionRate,
                address stakingToken,
                uint32 activeValidatorsLength,
                uint32 epochBlockInterval,
                uint32 undelegatePeriod,
                uint256 minValidatorStakeAmount,
                uint256 minStakingAmount,
                uint64 dposActivationBlock,
                uint256 minUndelegateBlocks,
                address blendReserve
            ) external;
            function setProductionLivenessDisabled(bool value) external;
            function setBlendStipendPerEpoch(uint256 value) external;
            function commitEpochCommittee() external;
        }
        interface IStakingPool {
            function initialize(address initialOwner) external;
        }
        interface IFluentGovernance {
            function initialize(address initialOwner, uint32 initialVotingPeriod) external;
        }
        interface IERC20 {
            function approve(address spender, uint256 value) external returns (bool);
            function transfer(address to, uint256 value) external returns (bool);
            function balanceOf(address account) external view returns (uint256);
        }
    }
}

pub fn run(
    keys: &KeySet,
    artefacts: &Artefacts,
    chain_id: u64,
    params: &BootstrapParams,
) -> eyre::Result<PredeployState> {
    // The genesis committee is `validators[0..committee_size]` — the FIRST K by
    // identity index, which is what `keys::derive` produces (every key is
    // `H(seed|role|i)`, pushed in `0..peers` order with `idx: i`, and nothing sorts
    // afterwards) and what the harness runs containers for (`validator-0..K-1`,
    // `owner-N.hex`, `addresses.json[N]`). The contract re-sorts the committee on peer
    // pubkey at commit time, so a member's identity index is NOT its committee
    // position — but membership is exactly {0..K-1} either way, and the node derives
    // its participant index from the same peer-pubkey order, so nothing downstream
    // reads identity index as position.
    let seated = params.committee_size.unwrap_or(keys.validators.len());
    eyre::ensure!(
        seated >= 1 && seated <= keys.validators.len(),
        "committee size {seated} outside the derived pool of {}",
        keys.validators.len()
    );
    let committee = &keys.validators[..seated];
    let deployer = keys.governance_signer.address();
    let mut ctx = bootstrap_ctx(chain_id, deployer);

    // `StakingContext`'s six-address tuple. The former ChainConfig predeploy is part
    // of the staking module now, so its slot resolves to STAKING_ADDR; the
    // SystemReward contract has no successor at all, and STAKING_ADDR stands in
    // there too rather than zero — a call to a codeless account returns Success, so
    // a zero would turn any future dereference into a silent no-op instead of an
    // error. `StakingPool` never dereferences the slot today (it only re-exposes it
    // through the inherited `getSystemReward()` getter).
    let pool_constructor = (
        STAKING_ADDR,
        STAKING_ADDR,
        STAKING_POOL_ADDR,
        GOVERNANCE_ADDR,
        STAKING_ADDR,
        STAKING_TOKEN_ADDR,
    )
        .abi_encode_sequence();

    // MockBlendToken: constructor mints to deployer → storage MUST be
    // copied (balanceOf, totalSupply). The two UUPS impls below set ONLY
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
        &artefacts.staking_pool,
        STAKING_POOL_ADDR,
        &pool_constructor,
        false,
    )?;
    deploy_governance(&mut ctx, deployer, &artefacts.governance)?;

    // Everything below runs through the rWasm executor, which rejects bare legacy
    // bytecode (`execute_rwasm_frame` returns `NotSupportedBytecode` for anything
    // that is neither rWasm-native nor an OwnableAccount). Re-install the three
    // Solidity predeploys in the production OwnableAccount(EVM_RUNTIME, ..) form
    // BEFORE the flip, or the first call into any of them halts.
    wrap_evm_predeploys(
        &mut ctx,
        &[STAKING_TOKEN_ADDR, STAKING_POOL_ADDR, GOVERNANCE_ADDR],
    )?;
    ctx.disabled_rwasm = false;
    ctx.add_bytecode(STAKING_ADDR, artefacts.staking_rwasm.clone());

    // The contract rejects a zero `minValidatorStakeAmount`/`minStakingAmount` and
    // `_addValidator` enforces `initialStake >= minValidatorStakeAmount`. Stakes must
    // also be `% BALANCE_COMPACT_PRECISION == 0` where the precision is `1e10`
    // (`consts.rs:336`). Pick 1 BLEND (1e18) as min stake + initial — a multiple of
    // 1e10, and smoke uses no real economics.
    let smoke_min_stake = U256::from(10u128).pow(U256::from(18));
    // `dposActivationBlock` defaults to `2 * epochBlockInterval` so the migration anchor
    // still lands in absolute epoch 2 (alignment invariant); both the interval and the
    // activation block MUST match the harness's `EPOCH_INTERVAL` / `DPOS_ACTIVATION_BLOCK`.
    //
    // The production-liveness tier's three parameters are NOT initializer arguments — they
    // are seeded from constants inside `apply_initial_config` and moved by governance, so a
    // soak that wants real verdicts lowers `minVerdictDueBlocks` rather than re-deploying.
    let env_u32 = |k: &str, default: u32| -> u32 {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let env_u64 = |k: &str, default: u64| -> u64 {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let epoch_block_interval = env_u32("EPOCH_BLOCK_INTERVAL", 32);
    let dpos_activation_block = params
        .dpos_activation_block
        .unwrap_or(2 * u64::from(epoch_block_interval));

    // `HEAVY_STAKE_MULT` (default 1 ⇒ byte-identical equal-stake genesis) skews
    // validator-0's genesis stake k× the others, so the weighted-VRF smoke can
    // assert it proposes proportionally more blocks. Committee membership is
    // unaffected (the seated set ≤ activeValidatorsLength ⇒ all selected),
    // and validator-0 stays ≥ minValidatorStakeAmount.
    let heavy_mult = U256::from(env_u64("HEAVY_STAKE_MULT", 1));
    let mut initial_stakes = vec![smoke_min_stake; seated];
    if let Some(first) = initial_stakes.first_mut() {
        *first *= heavy_mult;
    }
    let total_stake = initial_stakes
        .iter()
        .copied()
        .fold(U256::ZERO, |acc, s| acc + s);
    let validator_addrs: Vec<Address> = committee.iter().map(|v| v.l2_signer.address()).collect();

    let mut bls_pubkeys = Vec::with_capacity(seated);
    let mut bls_pops = Vec::with_capacity(seated);
    let mut peer_pubkeys = Vec::with_capacity(seated);
    for v in committee {
        let p = pop::produce(&v.bls, keys.chain_id)?;
        bls_pubkeys.push(Bytes::copy_from_slice(&p.bls_pubkey_uncompressed));
        bls_pops.push(Bytes::copy_from_slice(&p.bls_pop_uncompressed));
        use commonware_codec::Encode as _;
        let pk = commonware_cryptography::Signer::public_key(&v.peer).encode();
        peer_pubkeys.push(B256::from_slice(pk.as_ref()));
    }

    // MockBlendToken's constructor mints the whole supply to `msg.sender`, i.e. the
    // deployer. That is only useful where the deployer is also the account that later
    // spends BLEND; the sim/soak signs every funding and delegation with
    // `validator-0`'s owner key, so the supply has to move there or it sits on an
    // account nothing can spend from. Moving it makes that account the BLEND role
    // wholesale: stake sponsor, approver, and stipend reserve.
    let blend_holder = params.blend_holder.unwrap_or(deployer);
    if blend_holder != deployer {
        let supply_out = call_returning(
            &mut ctx,
            deployer,
            STAKING_TOKEN_ADDR,
            abi::IERC20::balanceOfCall { account: deployer }
                .abi_encode()
                .into(),
            "BLEND.balanceOf(deployer)",
        )?;
        let supply = U256::abi_decode(&supply_out).wrap_err("decode BLEND balanceOf")?;
        let hand_over = abi::IERC20::transferCall {
            to: blend_holder,
            value: supply,
        }
        .abi_encode();
        call_or_die(
            &mut ctx,
            deployer,
            STAKING_TOKEN_ADDR,
            hand_over.into(),
            "BLEND.transfer(holder)",
        )?;
    }

    // The BLEND stipend is drawn with `transferFrom(blendReserve, claimant, amount)` —
    // straight to whoever claims, never onto the staking contract (the epoch close
    // only writes credits; `staking.rs::pay_stipend`). The reserve is whatever account
    // holds the pot and has approved the staking contract, and it is also what the
    // close READS to decide whether the epoch is affordable at all: below the pot the
    // epoch closes at zero permanently. `apply_initial_config` refuses a zero reserve.
    // One allowance covers both draws: the genesis stakes `initialize` pulls, and the
    // stipend budget.
    let stipend_budget = U256::from(env_u64("BLEND_RESERVE_GENESIS_BLEND", 1_000_000))
        * U256::from(10u128).pow(U256::from(18));
    let approve_call = abi::IERC20::approveCall {
        spender: STAKING_ADDR,
        value: total_stake + stipend_budget,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        blend_holder,
        STAKING_TOKEN_ADDR,
        approve_call.into(),
        "BLEND.approve(staking)",
    )?;

    // One call seeds the chain configuration, the dependency addresses and every
    // genesis validator with its stake and verified consensus keys. There is no
    // verifier argument any more and no setter that could supply one: the module
    // verifies each proof of possession itself, against the fixed EIP-2537
    // precompile addresses.
    let staking_init = abi::IStaking::initializeCall {
        initialStakeOwner: blend_holder,
        validators: validator_addrs,
        initialStakes: initial_stakes,
        blsPubkeysUncompressed: bls_pubkeys,
        blsPopsUncompressed: bls_pops,
        peerPubkeys: peer_pubkeys,
        commissionRate: 0,
        stakingToken: STAKING_TOKEN_ADDR,
        activeValidatorsLength: seated as u32,
        epochBlockInterval: epoch_block_interval,
        undelegatePeriod: 16,
        minValidatorStakeAmount: smoke_min_stake,
        minStakingAmount: smoke_min_stake,
        dposActivationBlock: dpos_activation_block,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: blend_holder,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        deployer,
        STAKING_ADDR,
        staking_init.into(),
        "Staking.initialize",
    )?;

    // `apply_initial_config` writes `productionLivenessDisabled = true` deliberately —
    // the tier ships off, and an unwritten slot would ship it on. The devnet wants it
    // ON, so the flip is a governance call the initializer cannot make.
    let enable_liveness =
        abi::IStaking::setProductionLivenessDisabledCall { value: false }.abi_encode();
    call_or_die(
        &mut ctx,
        GOVERNANCE_ADDR,
        STAKING_ADDR,
        enable_liveness.into(),
        "Staking.setProductionLivenessDisabled(false)",
    )?;

    // Turn the per-epoch BLEND stipend ON for the devnet (0 = OFF kill-switch). Flat
    // pro-rata by stake among committee members that met the participation floor.
    let stipend_per_epoch = U256::from(env_u64("BLEND_STIPEND_PER_EPOCH_BLEND", 10))
        * U256::from(10u128).pow(U256::from(18));
    let set_stipend = abi::IStaking::setBlendStipendPerEpochCall {
        value: stipend_per_epoch,
    }
    .abi_encode();
    call_or_die(
        &mut ctx,
        GOVERNANCE_ADDR,
        STAKING_ADDR,
        set_stipend.into(),
        "Staking.setBlendStipendPerEpoch",
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

    init_governance(&mut ctx, deployer)?;

    // The committee for epoch 0. The contract selects it from its own registry and
    // sorts it on peer pubkey, which IS the consensus index space — there is nothing
    // for this side to supply or to agree with.
    let commit = abi::IStaking::commitEpochCommitteeCall {}.abi_encode();
    call_or_die(
        &mut ctx,
        SYSTEM_CALLER,
        STAKING_ADDR,
        commit.into(),
        "commitEpochCommittee[epoch=0]",
    )?;
    // No genesis beacon key is committed: the beacon is always-on live DKG and is
    // consumed internally (the per-block seed rides the consensus cert; there is no
    // on-chain PK_E — that layer was removed, DPOS_ARCHITECTURE §8.11).

    Ok(snapshot(
        &mut ctx,
        &[
            STAKING_ADDR,
            STAKING_POOL_ADDR,
            GOVERNANCE_ADDR,
            STAKING_TOKEN_ADDR,
        ],
    ))
}

/// Everything the `bare` arm installs: `FluentGovernance` at `GENESIS_GOVERNANCE`,
/// initialized, and nothing else — the staking address stays codeless, which is the
/// premise the stand is built on and what the reader's code-presence probe keys off.
///
/// The Governor is not the staking module; it is the control plane the module requires
/// to exist at a fixed address. The module compiles `GENESIS_GOVERNANCE` in as the sole
/// caller its privileged setters accept, and the stand's bring-up has three
/// governance-gated steps after the module lands by runtime-upgrade
/// (`setProductionLivenessDisabled`, `setBlendStipendPerEpoch`, `setDposActivationBlock`).
/// A Governor cannot be forge-created into place afterwards: CREATE derives its address
/// from the deployer's nonce, and this one is fixed in compiled bytecode. Same
/// reasoning as the runtime-upgrade owner slot `genesis::assemble` seeds on this arm.
///
/// No `disabled_rwasm` flip here, unlike [`run`]: the flip exists so the init sequence
/// can CALL the rWasm staking module, and there is no module on this arm. The Governor
/// is deployed and initialized on the mainnet-revm path and wrapped into production
/// `OwnableAccount` form by [`snapshot`], which is how every predeploy reached genesis
/// before the module existed.
pub fn run_governance_only(
    keys: &KeySet,
    governance_init_bytecode: &Bytes,
    chain_id: u64,
) -> eyre::Result<PredeployState> {
    let deployer = keys.governance_signer.address();
    let mut ctx = bootstrap_ctx(chain_id, deployer);
    deploy_governance(&mut ctx, deployer, governance_init_bytecode)?;
    init_governance(&mut ctx, deployer)?;
    Ok(snapshot(&mut ctx, &[GOVERNANCE_ADDR]))
}

/// The in-process EVM both arms deploy through. Shared so the Governor they each
/// produce is byte-identical — it is the one account they have in common, and the
/// production-path stand runs a chain that took it from `bare` and everything else from
/// a later delivery.
fn bootstrap_ctx(chain_id: u64, deployer: Address) -> EvmTestingContext {
    // PRECOMPILE_EVM_RUNTIME needs to be registered before any plain
    // EVM (`deployedBytecode`) deploy through `deploy_evm_tx` — without
    // it the EVM aborts with `MalformedBuiltinParams`. Mirrors the
    // e2e/src/lib.rs `with_full_genesis` trait impl.
    let fluent_contracts: Vec<_> = fluentbase_genesis::GENESIS_CONTRACTS_BY_ADDRESS
        .values()
        .cloned()
        .collect();
    let mut ctx = EvmTestingContext::default().with_contracts(&fluent_contracts);
    // Solidity predeploys are CREATE'd through the mainnet revm path, the only one
    // that accepts plain legacy initcode. `run` flips to the rWasm path afterwards,
    // before the staking module is installed; `run_governance_only` never needs to.
    ctx.disabled_rwasm = true;
    ctx.cfg.limit_contract_code_size = Some(usize::MAX);
    ctx.cfg.limit_contract_initcode_size = Some(usize::MAX);
    // EIP-3607 (RejectCallerWithCode) blocks tx where caller already has
    // code. We need to spoof caller = GOVERNANCE_ADDR (a deployed
    // contract) to satisfy the staking module's `ensure_governance`
    // (checks `contract_caller() == GENESIS_GOVERNANCE`, no code-shape check).
    // Same applies to SYSTEM_CALLER for `commitEpochCommittee`. Disable 3607
    // for the in-process bootstrap session only — not a real chain.
    ctx.cfg.disable_eip3607 = true;
    // block.chainid drives the contract's `fluent_namespace()` (= "FLUENT_DPOS_V1_"
    // ‖ u64 BE chain_id), which is both the PoP-signed message and the slashing
    // namespace. The Rust-side PoP is signed with `fluent_namespace(chain_id)` —
    // both MUST agree, else verifier.verify returns false →
    // ERR_INVALID_PROOF_OF_POSSESSION.
    ctx.cfg.chain_id = chain_id;
    // TxBuilder::create / TxBuilder::call leave tx.chain_id at its
    // TxEnv::default() value of Some(1), which then disagrees with our
    // cfg.chain_id = 2026 and trips the EIP-155 chain-ID check. We don't
    // care about replay protection in an in-process bootstrap session,
    // so disable the check entirely instead of patching each TxEnv.
    ctx.cfg.tx_chain_id_check = false;
    ctx.add_balance(deployer, U256::from(10u128).pow(U256::from(22)));
    ctx
}

/// `FluentGovernance`'s constructor takes `(IStaking, IChainConfig)` and only ASSIGNS
/// both to immutables before `_disableInitializers()` — it dereferences neither, and
/// `initialize` calls nothing but OZ `__*_init` (verified in
/// `contracts/governance/FluentGovernance.sol:40-53`). The one dereference,
/// `onlyValidatorOwner`, guards proposal creation at runtime. So the Governor can be
/// installed before the staking module exists, which is exactly what `bare` does. Both
/// addresses resolve to the staking module: it absorbed the ChainConfig predeploy.
fn deploy_governance(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    init_bytecode: &Bytes,
) -> eyre::Result<()> {
    let constructor = (STAKING_ADDR, STAKING_ADDR).abi_encode_sequence();
    deploy_to_canonical(
        ctx,
        deployer,
        init_bytecode,
        GOVERNANCE_ADDR,
        &constructor,
        false,
    )
}

fn init_governance(ctx: &mut EvmTestingContext, deployer: Address) -> eyre::Result<()> {
    let gov_init = abi::IFluentGovernance::initializeCall {
        initialOwner: deployer,
        initialVotingPeriod: GOVERNANCE_VOTING_PERIOD_BLOCKS,
    }
    .abi_encode();
    call_or_die(
        ctx,
        deployer,
        GOVERNANCE_ADDR,
        gov_init.into(),
        "FluentGovernance.initialize",
    )
}

fn deploy_to_canonical(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    init_bytecode: &Bytes,
    canonical: Address,
    constructor_args: &[u8],
    copy_storage: bool,
) -> eyre::Result<()> {
    let mut init = init_bytecode.to_vec();
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

/// Re-install each address's code in the production
/// `OwnableAccount(EVM_RUNTIME, EthereumMetadata)` form, in place.
///
/// `add_bytecode` rebuilds the whole `AccountInfo`, so the balance is read first and
/// re-applied; storage lives beside `info` on the cache entry and survives the
/// rebuild (`CacheDB::insert_account_info` only calls `update_info`).
fn wrap_evm_predeploys(ctx: &mut EvmTestingContext, addrs: &[Address]) -> eyre::Result<()> {
    for addr in addrs {
        let account = ctx
            .db
            .cache
            .accounts
            .get(addr)
            .ok_or_else(|| eyre::eyre!("predeploy {addr:?} absent before the rWasm flip"))?;
        let balance = account.info.balance;
        let raw = account
            .info
            .code
            .as_ref()
            .ok_or_else(|| eyre::eyre!("predeploy {addr:?} has no code before the rWasm flip"))?
            .original_bytes();
        ctx.add_bytecode(*addr, wrap_for_rwasm(raw));
        ctx.add_balance(*addr, balance);
    }
    Ok(())
}

/// Wrap deployed EVM runtime bytecode in `OwnableAccount(EVM_RUNTIME, code)`, the
/// shape fluent's executor requires: every contract must be either rWasm-native or an
/// OwnableAccount delegating to a runtime precompile, else `execute_rwasm_frame`
/// returns `NotSupportedBytecode`. During normal CREATE through fluent's rWasm EVM
/// revm produces this wrapping itself; the mainnet-revm CREATE path this bootstrap
/// deploys through does not, so we replicate it. Already-wrapped (`0xEF44`) and
/// rWasm-native (`0xEF52`) code passes through untouched.
fn wrap_for_rwasm(raw: Bytes) -> Bytes {
    if raw.starts_with(&[0xEF, 0x44]) || raw.starts_with(&[0xEF, 0x52]) {
        return raw;
    }
    // EVM_RUNTIME reads metadata as `EthereumMetadata` (see
    // crates/evm/src/metadata.rs:23) — `[code_hash 32 bytes] || [bytecode]`
    // for legacy. Without the 32-byte hash prefix EVM_RUNTIME interprets the
    // first 32 bytes of EVM bytecode as the hash, skips them, runs the rest,
    // halts with StackOverflow.
    let metadata = fluentbase_evm::EthereumMetadata::new_legacy(raw).write_to_bytes();
    let mut buf = Vec::with_capacity(23 + metadata.len());
    buf.extend_from_slice(&[0xEF, 0x44, 0x00]);
    buf.extend_from_slice(fluentbase_types::PRECOMPILE_EVM_RUNTIME.as_slice());
    buf.extend_from_slice(&metadata);
    buf.into()
}

fn call_returning(
    ctx: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Bytes,
    label: &str,
) -> eyre::Result<Bytes> {
    let res = ctx.call_evm_tx(caller, callee, input, Some(BOOTSTRAP_GAS_LIMIT), None);
    if !res.is_success() {
        return Err(eyre::eyre!(
            "{label} (caller={caller:?} → {callee:?}) reverted: {res:?}"
        ));
    }
    Ok(res.output().cloned().unwrap_or_default())
}

fn call_or_die(
    ctx: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Bytes,
    label: &str,
) -> eyre::Result<()> {
    call_returning(ctx, caller, callee, input, label).map(|_| ())
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
            bytecode_by_address.insert(*addr, wrap_for_rwasm(bytecode.original_bytes()));
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
