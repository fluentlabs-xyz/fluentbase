//! [`RethBlockDeriver`] — the node-side [`DerivedBlockBuilder`]: executes an
//! agreed `OrderBlock`'s txs on the parent's post-state via reth-evm's
//! `BlockBuilder` (the stock payload builder's execution path) and seals the
//! derived Ethereum block. Determinism contract: every input is agreed data
//! (the OrderBlock fields) or derived state — never local config.

use alloy_consensus::Header;
use alloy_primitives::B256;
use eyre::WrapErr as _;
use std::sync::Arc;
use fluentbase_consensus::beacon::{
    seed::{prev_randao_for_round, GroupPublic},
    types::Seed,
};
use fluentbase_consensus::{DerivedBlock, DerivedBlockBuilder, OrderBlock, ParentHeaderMissing};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{
    execute::{BlockBuilder as _, BlockExecutionError, BlockExecutionOutput, BlockValidationError},
    ConfigureEvm, NextBlockEnvAttributes,
};
use reth_primitives_traits::{RecoveredBlock, SealedHeader, SignedTransaction as _};
use reth_revm::{database::StateProviderDatabase, db::State};
use reth_storage_api::{HeaderProvider, StateProviderFactory};
use reth_trie_common::{updates::TrieUpdates, HashedPostState};

/// One derivation's full output: the recovered block plus every execution
/// artifact the engine-tree needs to import it WITHOUT re-executing
/// (`EngineApiRequest::InsertExecutedBlock`). The consensus crate sees it
/// only through [`DerivedBlock`] (hash + number).
#[derive(Debug)]
pub struct DerivedExecution {
    pub recovered: RecoveredBlock<reth_ethereum_primitives::Block>,
    pub output: BlockExecutionOutput<reth_ethereum_primitives::Receipt>,
    pub hashed_state: HashedPostState,
    pub trie_updates: TrieUpdates,
}

impl DerivedBlock for DerivedExecution {
    fn evm_hash(&self) -> B256 {
        self.recovered.hash()
    }

    fn number(&self) -> u64 {
        self.recovered.number
    }
}

/// Resolves the beacon group key `PK_E` for the epoch of a block being derived,
/// reading it at the executed parent's state hash. Returns `None` ⇒ gated
/// fallback (`prev_randao = order.digest()`). Per-epoch: rotated keys are read
/// trustlessly from L2 state (`getEpochBeaconKey`); pre-rotation / uncommitted
/// epochs fall back to the genesis `PK_0`.
type PkResolver = Arc<dyn Fn(u64, B256) -> Option<GroupPublic> + Send + Sync>;

/// Beacon verify material: a per-epoch group-key resolver + the seed namespace
/// the deriver checks a cert-recovered seed against. `None` (the
/// [`RethBlockDeriver::new`] default) keeps the pre-beacon behaviour —
/// `prev_randao = order.digest()`, the gated `assurance=false` fallback. The
/// seed itself is NOT held here: it is supplied per block by the caller
/// (extracted from the consensus certificate), so resolution is synchronous.
#[derive(Clone)]
struct BeaconVerify {
    /// `chain_namespace ‖ "_BEACON_SEED"` — verifies the seed vs `PK_E`.
    namespace: Vec<u8>,
    /// Resolves `PK_E` for `(epoch, parent_state_hash)`.
    pk_for_epoch: PkResolver,
}

impl std::fmt::Debug for BeaconVerify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeaconVerify")
            .field("namespace", &self.namespace)
            .field("pk_for_epoch", &"<resolver>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct RethBlockDeriver<Client, Evm> {
    client: Client,
    evm_config: Evm,
    beacon: Option<BeaconVerify>,
}

impl<Client, Evm> RethBlockDeriver<Client, Evm> {
    /// Pre-beacon constructor: `prev_randao` is always the weak deterministic
    /// fallback (`order.digest()`). The verified randomness path is opted into
    /// via [`Self::with_beacon_resolver`].
    pub fn new(client: Client, evm_config: Evm) -> Self {
        Self {
            client,
            evm_config,
            beacon: None,
        }
    }

    /// Attach the per-epoch beacon-key resolver + seed namespace used to verify
    /// a cert-recovered seed. The resolver reads `PK_E` for the block's epoch
    /// from L2 state (genesis `PK_0` fallback); a verified seed then yields
    /// `prev_randao = H(seed)`, else the gated `order.digest()` fallback.
    pub fn with_beacon_resolver(mut self, namespace: Vec<u8>, pk_for_epoch: PkResolver) -> Self {
        self.beacon = Some(BeaconVerify {
            namespace,
            pk_for_epoch,
        });
        self
    }

    /// Decide `prev_randao`: the gated threshold value `H(seed)` when the
    /// caller supplied a seed (recovered from the block's consensus
    /// certificate) that verifies against the epoch's `PK_E`, else the weak
    /// deterministic `fallback`. The epoch is taken from the seed's round; the
    /// key is resolved at `parent_state_hash`. Synchronous — seed in hand.
    fn resolve_prev_randao(
        &self,
        seed: Option<&Seed>,
        parent_state_hash: B256,
        fallback: B256,
    ) -> B256 {
        let (Some(beacon), Some(s)) = (&self.beacon, seed) else {
            return fallback;
        };
        let pk_epoch = (beacon.pk_for_epoch)(s.target_round.epoch().get(), parent_state_hash);
        let (prev_randao, assurance) = prev_randao_for_round(
            s.target_round,
            Some(s),
            pk_epoch.as_ref(),
            &beacon.namespace,
            fallback,
        );
        if assurance {
            tracing::info!(round = ?s.target_round, %prev_randao, "beacon: threshold prev_randao active");
        }
        prev_randao
    }
}

impl<Client, Evm> DerivedBlockBuilder for RethBlockDeriver<Client, Evm>
where
    Client: StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>
        + Clone
        + 'static,
{
    type Derived = DerivedExecution;

    async fn derive_and_execute(
        &self,
        order: OrderBlock,
        parent_evm_hash: B256,
        seed: Option<Seed>,
    ) -> eyre::Result<DerivedExecution> {
        // The seed (if any) was recovered from this block's consensus
        // certificate by the caller, so prev_randao resolution is synchronous;
        // execution itself is CPU-bound and stays on a blocking thread.
        let prev_randao =
            self.resolve_prev_randao(seed.as_ref(), parent_evm_hash, order.digest().0);
        let client = self.client.clone();
        let evm_config = self.evm_config.clone();
        // EVM execution + state-root computation are CPU-bound (~V per
        // block); keep them off the async worker threads.
        tokio::task::spawn_blocking(move || {
            derive_sync(&client, &evm_config, &order, parent_evm_hash, prev_randao)
        })
        .await
        .wrap_err("derive task panicked")?
    }
}

fn derive_sync<Client, Evm>(
    client: &Client,
    evm_config: &Evm,
    order: &OrderBlock,
    parent_evm_hash: B256,
    prev_randao: B256,
) -> eyre::Result<DerivedExecution>
where
    Client: StateProviderFactory + HeaderProvider<Header = Header>,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
{
    let parent_header = client
        .header(parent_evm_hash)
        .wrap_err("read parent header")?
        .ok_or(ParentHeaderMissing(parent_evm_hash))?;
    let parent_sealed = SealedHeader::new(parent_header, parent_evm_hash);

    // The header read above just proved the parent exists, so a state-read
    // failure for the same hash is (overwhelmingly) the same eager-canonicalization
    // visibility lag — type it so the recovery/jump retry can absorb it; the
    // underlying provider error stays in the chain for the timeout report.
    let state_provider = client
        .state_by_block_hash(parent_evm_hash)
        .map_err(|e| eyre::Report::new(e).wrap_err(ParentHeaderMissing(parent_evm_hash)))?;
    let mut db = State::builder()
        .with_database(StateProviderDatabase::new(state_provider.as_ref()))
        .with_bundle_update()
        .build();

    // Field mapping mirrors the live chain's attrs builder
    // (`FluentPayloadAttributesBuilder::build_attrs`) except the
    // node-local values it used: prev_randao (was `B256::random()`) is the
    // beacon-resolved value (`H(seed(h))` or the gated `order.digest()`
    // fallback, decided by the caller), and timestamp/fee_recipient/gas_limit
    // come from the agreed OrderBlock.
    let attrs = NextBlockEnvAttributes {
        timestamp: order.timestamp,
        suggested_fee_recipient: order.fee_recipient,
        prev_randao,
        gas_limit: order.gas_limit,
        parent_beacon_block_root: Some(B256::ZERO),
        withdrawals: None,
        extra_data: order.extra_data.clone(),
        slot_number: None,
    };

    let mut builder = evm_config
        .builder_for_next_block(&mut db, &parent_sealed, attrs)
        .map_err(|e| eyre::eyre!("builder_for_next_block: {e}"))?;
    builder.apply_pre_execution_changes()?;

    for tx in &order.txs {
        let recovered = match tx.try_clone_into_recovered() {
            Ok(recovered) => recovered,
            // Deterministic skip: recovery is a pure function of the tx
            // bytes, so every node skips the same txs. Unreachable behind an
            // honest quorum (verify rejects unrecoverable signatures), kept
            // for byzantine-agreed artifacts.
            Err(error) => {
                tracing::warn!(%error, "derive: skipping unrecoverable tx");
                continue;
            }
        };
        // EXACT copy of the stock builder's skip rule
        // (ethereum/payload/src/lib.rs:370-407) minus the pool bookkeeping —
        // diverging from it would fork derived blocks between nodes.
        match builder.execute_transaction(recovered) {
            Ok(_) => {}
            Err(BlockExecutionError::Validation(BlockValidationError::InvalidTx { .. }))
            | Err(BlockExecutionError::Validation(
                BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas { .. },
            )) => continue,
            Err(fatal) => return Err(fatal.into()),
        }
    }

    let outcome = builder.finish(&state_provider, None)?;
    let state = db.take_bundle();
    Ok(DerivedExecution {
        recovered: outcome.block,
        output: BlockExecutionOutput {
            result: outcome.execution_result,
            state,
        },
        hashed_state: outcome.hashed_state,
        trie_updates: outcome.trie_updates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::{FluentEvmConfig, FluentEvmFactory};
    use alloy_consensus::{SignableTransaction as _, TxEip1559};
    use alloy_genesis::GenesisAccount;
    use alloy_primitives::{Address, Bytes, TxKind, U256};
    use alloy_signer::SignerSync as _;
    use alloy_signer_local::PrivateKeySigner;
    use fluentbase_consensus::Digest;
    use reth_chainspec::{
        make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, DEV_HARDFORKS,
    };
    use reth_db_common::init::init_genesis;
    use reth_ethereum_primitives::{Transaction, TransactionSigned};
    use reth_provider::test_utils::create_test_provider_factory_with_chain_spec;
    use std::sync::Arc;

    const DEV_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn signed_transfer(signer: &PrivateKeySigner, nonce: u64) -> TransactionSigned {
        let tx = TxEip1559 {
            chain_id: 1337,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(Address::repeat_byte(0x55)),
            value: U256::from(1u64),
            ..Default::default()
        };
        let sig = signer.sign_hash_sync(&tx.signature_hash()).expect("sign");
        TransactionSigned::new_unhashed(Transaction::Eip1559(tx), sig)
    }

    // Determinism is THE property the committee's `result` agreement rests
    // on: two independent derivations of the same (order, parent) must be
    // byte-identical, including the deterministic skip of an
    // invalid-at-its-turn tx (nonce gap).
    #[test]
    fn derive_is_deterministic_and_skips_invalid_txs() {
        let signer: PrivateKeySigner = DEV_KEY.parse().expect("key");
        let genesis = fluentbase_genesis::local_genesis_from_file().extend_accounts([(
            signer.address(),
            GenesisAccount::default().with_balance(U256::from(10u64).pow(U256::from(18u64))),
        )]);
        let hardforks = DEV_HARDFORKS.clone();
        let chain_spec: Arc<ChainSpec> = Arc::new(ChainSpec {
            chain: Chain::from(1337u64),
            genesis_header: reth_primitives_traits::SealedHeader::new_unhashed(
                make_genesis_header(&genesis, &hardforks),
            ),
            genesis,
            paris_block_and_final_difficulty: Some((0, U256::ZERO)),
            hardforks,
            base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
            deposit_contract: None,
            ..Default::default()
        });

        let factory = create_test_provider_factory_with_chain_spec(chain_spec.clone());
        let genesis_hash = init_genesis(&factory).expect("init genesis");
        let provider =
            reth_provider::providers::BlockchainProvider::new(factory).expect("provider");

        let genesis_header = chain_spec.genesis_header();
        let order = OrderBlock {
            parent: Digest(B256::ZERO),
            height: genesis_header.number + 1,
            timestamp: genesis_header.timestamp + 1,
            fee_recipient: Address::repeat_byte(0x77),
            gas_limit: genesis_header.gas_limit,
            extra_data: Bytes::from(vec![0xAB, 0xCD]),
            result: B256::ZERO,
            txs: vec![signed_transfer(&signer, 0), signed_transfer(&signer, 7)],
            beacon_outcome: None,
            beacon_seed: None,
        };

        let evm_config = FluentEvmConfig::new(
            chain_spec.clone(),
            FluentEvmFactory::default(),
            Address::ZERO,
            Address::ZERO,
            Address::ZERO,
        );

        let prev_randao = B256::repeat_byte(0x42);
        let a = derive_sync(&provider, &evm_config, &order, genesis_hash, prev_randao)
            .expect("derive a");
        let b = derive_sync(&provider, &evm_config, &order, genesis_hash, prev_randao)
            .expect("derive b");

        assert_eq!(
            a.evm_hash(),
            b.evm_hash(),
            "derivation must be byte-identical"
        );
        let a = a.recovered.into_sealed_block();
        // nonce-7 (gap) deterministically skipped; nonce-0 included.
        assert_eq!(a.body().transactions.len(), 1);
        // Agreed-field mapping into the derived header.
        assert_eq!(a.header().beneficiary, order.fee_recipient);
        assert_eq!(a.header().timestamp, order.timestamp);
        assert_eq!(a.header().gas_limit, order.gas_limit);
        assert_eq!(a.header().extra_data, order.extra_data);
        // prev_randao is the caller-resolved value, not the ordering digest.
        assert_eq!(a.header().mix_hash, prev_randao);
    }

    // The derive-gate (Decision C4): with a beacon source whose cache holds a
    // verifiable seed(h) for the target height, prev_randao(h) = H(seed(h));
    // otherwise (height absent, or no beacon source) it degrades to the weak
    // deterministic fallback — never blocking, the Q4 assurance=false path.
    #[test]
    fn resolve_prev_randao_uses_verified_seed_else_falls_back() {
        use commonware_consensus::types::{Epoch, Round, View};
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use fluentbase_bls::beacon::{recover_seed, sign_seed_partial};
        use fluentbase_consensus::beacon::seed::{prev_randao_from_seed, seed_namespace};

        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        let ns = seed_namespace(b"fluent-devnet");
        let round = Round::new(Epoch::new(1), View::new(50));
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        let seed = Seed {
            target_round: round,
            signature: recover_seed(&sharing, &partials).expect("recover"),
        };
        let expected = prev_randao_from_seed(&seed);
        let fallback = B256::repeat_byte(0x99);

        // The inherent ctor/resolver have no Client/Evm bounds → unit types.
        // A constant resolver (every epoch → the same `PK_0`) reproduces the
        // pre-rotation static-key behaviour; `at` is ignored.
        let pk = *sharing.public();
        let resolver: PkResolver = Arc::new(move |_epoch, _at| Some(pk));
        let deriver = RethBlockDeriver::<(), ()>::new((), ()).with_beacon_resolver(ns, resolver);
        let at = B256::ZERO;

        // seed present + verifies → threshold randomness.
        assert_eq!(deriver.resolve_prev_randao(Some(&seed), at, fallback), expected);
        // a seed for a DIFFERENT round → gated fallback (round mismatch).
        let other = Seed {
            target_round: Round::new(Epoch::new(1), View::new(51)),
            signature: seed.signature,
        };
        assert_eq!(
            deriver.resolve_prev_randao(Some(&other), at, fallback),
            fallback
        );
        // no seed → fallback.
        assert_eq!(deriver.resolve_prev_randao(None, at, fallback), fallback);
        // no beacon source at all → fallback.
        let plain = RethBlockDeriver::<(), ()>::new((), ());
        assert_eq!(plain.resolve_prev_randao(Some(&seed), at, fallback), fallback);
    }
}
