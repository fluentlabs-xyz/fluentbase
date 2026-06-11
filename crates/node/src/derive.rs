//! [`RethBlockDeriver`] — the node-side [`DerivedBlockBuilder`]: executes an
//! agreed `OrderBlock`'s txs on the parent's post-state via reth-evm's
//! `BlockBuilder` (the stock payload builder's execution path) and seals the
//! derived Ethereum block. Determinism contract: every input is agreed data
//! (the OrderBlock fields) or derived state — never local config.

use alloy_consensus::Header;
use alloy_primitives::B256;
use eyre::WrapErr as _;
use fluentbase_consensus::{DerivedBlock, DerivedBlockBuilder, OrderBlock};
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

#[derive(Clone, Debug)]
pub struct RethBlockDeriver<Client, Evm> {
    client: Client,
    evm_config: Evm,
}

impl<Client, Evm> RethBlockDeriver<Client, Evm> {
    pub fn new(client: Client, evm_config: Evm) -> Self {
        Self { client, evm_config }
    }
}

impl<Client, Evm> DerivedBlockBuilder for RethBlockDeriver<Client, Evm>
where
    Client: StateProviderFactory
        + HeaderProvider<Header = Header>
        + Clone
        + Send
        + Sync
        + 'static,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>
        + Clone
        + 'static,
{
    type Derived = DerivedExecution;

    async fn derive_and_execute(
        &self,
        order: OrderBlock,
        parent_evm_hash: B256,
    ) -> eyre::Result<DerivedExecution> {
        let client = self.client.clone();
        let evm_config = self.evm_config.clone();
        // EVM execution + state-root computation are CPU-bound (~V per
        // block); keep them off the async worker threads.
        tokio::task::spawn_blocking(move || derive_sync(&client, &evm_config, &order, parent_evm_hash))
            .await
            .wrap_err("derive task panicked")?
    }
}

fn derive_sync<Client, Evm>(
    client: &Client,
    evm_config: &Evm,
    order: &OrderBlock,
    parent_evm_hash: B256,
) -> eyre::Result<DerivedExecution>
where
    Client: StateProviderFactory + HeaderProvider<Header = Header>,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
{
    let parent_header = client
        .header(parent_evm_hash)
        .wrap_err("read parent header")?
        .ok_or_else(|| eyre::eyre!("derive: parent header {parent_evm_hash} not found"))?;
    let parent_sealed = SealedHeader::new(parent_header, parent_evm_hash);

    let state_provider = client
        .state_by_block_hash(parent_evm_hash)
        .wrap_err("derive: parent state not available")?;
    let mut db = State::builder()
        .with_database(StateProviderDatabase::new(state_provider.as_ref()))
        .with_bundle_update()
        .build();

    // Field mapping mirrors the live chain's attrs builder
    // (`FluentPayloadAttributesBuilder::build_attrs`) except the two
    // node-local values it used: prev_randao (was `B256::random()`) becomes
    // the ordering digest, and the timestamp/fee_recipient/gas_limit come
    // from the agreed OrderBlock.
    let attrs = NextBlockEnvAttributes {
        timestamp: order.timestamp,
        suggested_fee_recipient: order.fee_recipient,
        prev_randao: order.digest().0,
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
        };

        let evm_config = FluentEvmConfig::new(
            chain_spec.clone(),
            FluentEvmFactory::default(),
            Address::ZERO,
            Address::ZERO,
        );

        let a = derive_sync(&provider, &evm_config, &order, genesis_hash).expect("derive a");
        let b = derive_sync(&provider, &evm_config, &order, genesis_hash).expect("derive b");

        assert_eq!(a.evm_hash(), b.evm_hash(), "derivation must be byte-identical");
        let a = a.recovered.into_sealed_block();
        // nonce-7 (gap) deterministically skipped; nonce-0 included.
        assert_eq!(a.body().transactions.len(), 1);
        // Agreed-field mapping into the derived header.
        assert_eq!(a.header().beneficiary, order.fee_recipient);
        assert_eq!(a.header().timestamp, order.timestamp);
        assert_eq!(a.header().gas_limit, order.gas_limit);
        assert_eq!(a.header().extra_data, order.extra_data);
    }
}
