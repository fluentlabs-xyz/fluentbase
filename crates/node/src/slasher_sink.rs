//! Production [`SlasherTxSink`] implementation: signs slash txs locally
//! with the slasher EOA key and submits them via the reth `TransactionPool`
//! (no HTTP RPC).
//!
//! Durability model (fix 5a). A reth receipt carries only a success bit, not
//! the revert reason, so revert classification cannot be done post-hoc. The
//! sink therefore **pre-flight-simulates** the slash call (an `eth_call`-style
//! execute-and-discard against latest state — the same composition the
//! staking-reader uses) BEFORE submitting:
//!
//! - simulation reverts with `AlreadySlashedForEquivocation` → the victim is
//!   already tombstoned; the goal is achieved, return [`SubmitOutcome::AlreadySlashed`]
//!   without spending gas;
//! - simulation reverts otherwise → a deterministic bug (calldata / EIP-2537
//!   encoding); return [`SubmitOutcome::Failed`] (no submit, loud log);
//! - simulation succeeds → submit the tx, await on-chain inclusion, read the
//!   receipt, and return [`SubmitOutcome::Mined`] only on `status == 1`.
//!
//! The WAL entry is acked by the consumer only on `Mined`/`AlreadySlashed`, so
//! evidence is never pruned before the slash is genuinely on-chain.

use std::time::Duration;

use alloy_consensus::{
    transaction::{Recovered, SignableTransaction},
    EthereumTxEnvelope, TxEip4844, TxLegacy, TxReceipt,
};
use alloy_evm::Evm;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolError};
use fluentbase_consensus::slasher::actor::{SlasherTxSink, SubmitOutcome};
use futures::StreamExt as _;
use reth_evm::ConfigureEvm;
use reth_primitives_traits::HeaderTy;
use reth_revm::{database::StateProviderDatabase, revm::context::result::ExecutionResult};
use reth_storage_api::{
    AccountReader, BlockNumReader, HeaderProvider, ReceiptProvider, StateProviderFactory,
};
use reth_transaction_pool::{
    PoolTransaction, TransactionEvent, TransactionOrigin, TransactionPool,
};
use tracing::{debug, warn};

/// Gas limit upper bound for slash submissions. Slash entry points iterate
/// over the equivocation evidence + pairing precompiles; conservatively
/// budget 2M gas per call.
///
/// NB (deferred): the pre-flight [`Self::simulate`] runs via
/// `transact_system_call`, which revm hard-codes at 30M gas
/// (revm-handler `system_call.rs`), NOT this 2M budget. So a slash that
/// consumes 2M–30M sims `WouldSucceed` but mines `status == 0` on the real
/// 2M-budget tx. This is fail-safe today (→ [`SubmitOutcome::Failed`] →
/// retry, never a wrong ack), so the sim's "would-succeed ⇒ will-succeed"
/// promise holds for the common case. A faithful cap would require replacing
/// the system-call sim with a funded `transact` carrying an explicit
/// `gas_limit` (and disabling balance/nonce checks for the ZERO caller);
/// deferred as it rewrites the tested sim path for a fail-safe edge.
const SLASH_GAS_LIMIT: u64 = 2_000_000;

/// Gas price used for slash txs (1 gwei). Slash txs are infrequent so a
/// flat legacy price keeps the implementation simple; gas-pricing
/// optimisation can be a follow-up.
const SLASH_GAS_PRICE: u128 = 1_000_000_000;

/// Upper bound on how long to await on-chain inclusion before treating the
/// submission as failed (retried after a restart). Pre-flight simulation has
/// already confirmed the tx would succeed, so inclusion should be prompt on a
/// ~1 block/sec chain; this only guards a wedged pool from blocking the
/// single-threaded slasher consumer forever.
const SLASH_INCLUSION_TIMEOUT: Duration = Duration::from_secs(120);

sol! {
    /// `Staking.sol` equivocation replay-guard revert. The 4-byte selector is
    /// matched against the pre-flight simulation's revert output to recognise
    /// an already-tombstoned victim.
    error AlreadySlashedForEquivocation(address validator);
}

pub struct PoolTxSink<P, Provider, E>
where
    P: TransactionPool,
    Provider: BlockNumReader + StateProviderFactory + Clone + Send + Sync + 'static,
    E: ConfigureEvm + Clone + Send + Sync + 'static,
{
    signer: PrivateKeySigner,
    signer_address: Address,
    chain_id: u64,
    pool: P,
    provider: Provider,
    evm_config: E,
}

impl<P, Provider, E> PoolTxSink<P, Provider, E>
where
    P: TransactionPool,
    Provider: BlockNumReader + StateProviderFactory + Clone + Send + Sync + 'static,
    E: ConfigureEvm + Clone + Send + Sync + 'static,
{
    pub fn new(
        signer: PrivateKeySigner,
        chain_id: u64,
        pool: P,
        provider: Provider,
        evm_config: E,
    ) -> Self {
        let signer_address = signer.address();
        Self {
            signer,
            signer_address,
            chain_id,
            pool,
            provider,
            evm_config,
        }
    }

    /// Compute the next-available nonce: max(state nonce, pending pool
    /// nonces). The pool tracks outstanding txs we've submitted earlier in
    /// this session that haven't mined yet — using state alone would
    /// collide their nonces.
    fn next_nonce(&self) -> eyre::Result<u64> {
        let state = self
            .provider
            .latest()
            .map_err(|e| eyre::eyre!("state provider lookup failed: {e:?}"))?;
        let state_nonce = state
            .basic_account(&self.signer_address)
            .map_err(|e| eyre::eyre!("basic_account lookup failed: {e:?}"))?
            .map(|acc| acc.nonce)
            .unwrap_or(0);
        let pending_max = self
            .pool
            .get_transactions_by_sender(self.signer_address)
            .into_iter()
            .map(|tx| tx.nonce())
            .max();
        let next = match pending_max {
            Some(p) if p + 1 > state_nonce => p + 1,
            _ => state_nonce,
        };
        Ok(next)
    }
}

/// Outcome of the pre-flight `eth_call`-style simulation of a slash call.
enum Sim {
    /// The call would succeed against latest state — safe to submit.
    WouldSucceed,
    /// Reverted with `AlreadySlashedForEquivocation` — already tombstoned.
    AlreadyTombstoned,
    /// Reverted/halted for any other reason — a deterministic bug.
    Rejected(String),
}

impl<P, Provider, E> PoolTxSink<P, Provider, E>
where
    P: TransactionPool,
    Provider: BlockNumReader
        + StateProviderFactory
        + HeaderProvider<Header = HeaderTy<E::Primitives>>
        + ReceiptProvider
        + Clone
        + Send
        + Sync
        + 'static,
    E: ConfigureEvm + Clone + Send + Sync + 'static,
{
    /// Execute `calldata` against `target` at the latest state and discard the
    /// state diff (eth_call semantics). Uses the system-call path (no caller
    /// funding / nonce / gas) — slash entry points are permissionless and do
    /// not gate on `msg.sender`. This is a full `TxKind::Call`, so the
    /// state-mutating `slashEquivocation*` executes normally and we observe
    /// success/revert without committing.
    fn simulate(&self, target: Address, calldata: Bytes) -> Result<Sim, String> {
        let num = self
            .provider
            .last_block_number()
            .map_err(|e| format!("last_block_number: {e:?}"))?;
        let header = self
            .provider
            .header_by_number(num)
            .map_err(|e| format!("header_by_number: {e:?}"))?
            .ok_or_else(|| format!("no header at block {num}"))?;
        let state = self
            .provider
            .latest()
            .map_err(|e| format!("latest state: {e:?}"))?;

        let db = StateProviderDatabase::new(state);
        let mut evm = self
            .evm_config
            .evm_for_block(db, &header)
            .map_err(|e| format!("evm_for_block: {e:?}"))?;

        let out = evm
            .transact_system_call(Address::ZERO, target, calldata)
            .map_err(|e| format!("transact_system_call: {e:?}"))?;

        Ok(match out.result {
            ExecutionResult::Success { .. } => Sim::WouldSucceed,
            ExecutionResult::Revert { output, .. } => {
                if output.len() >= 4 && output[..4] == AlreadySlashedForEquivocation::SELECTOR {
                    Sim::AlreadyTombstoned
                } else {
                    Sim::Rejected(format!(
                        "revert: 0x{}",
                        alloy_primitives::hex::encode(&output)
                    ))
                }
            }
            ExecutionResult::Halt { reason, .. } => Sim::Rejected(format!("halt: {reason:?}")),
        })
    }
}

impl<P, Provider, E> SlasherTxSink for PoolTxSink<P, Provider, E>
where
    P: TransactionPool<Transaction: PoolTransaction<Consensus = EthereumTxEnvelope<TxEip4844>>>
        + Clone
        + Send
        + Sync
        + 'static,
    Provider: BlockNumReader
        + StateProviderFactory
        + HeaderProvider<Header = HeaderTy<E::Primitives>>
        + ReceiptProvider
        + Clone
        + Send
        + Sync
        + 'static,
    E: ConfigureEvm + Clone + Send + Sync + 'static,
{
    fn submit<'a>(
        &'a self,
        target: Address,
        calldata: Bytes,
    ) -> std::pin::Pin<Box<dyn core::future::Future<Output = SubmitOutcome> + Send + 'a>> {
        Box::pin(async move {
            // 1. Pre-flight simulate against latest state (revert reason is only
            //    available here, not in a post-hoc receipt).
            match self.simulate(target, calldata.clone()) {
                Ok(Sim::WouldSucceed) => {}
                Ok(Sim::AlreadyTombstoned) => {
                    debug!(%target, "pre-flight: victim already tombstoned");
                    return SubmitOutcome::AlreadySlashed;
                }
                Ok(Sim::Rejected(why)) => {
                    return SubmitOutcome::Failed(format!(
                        "pre-flight simulation rejected slash ({why}) — deterministic bug, not submitting"
                    ));
                }
                Err(e) => {
                    return SubmitOutcome::Failed(format!("pre-flight simulation error: {e}"));
                }
            }

            // 2. Build + sign the tx.
            let nonce = match self.next_nonce() {
                Ok(n) => n,
                Err(e) => return SubmitOutcome::Failed(format!("nonce lookup: {e}")),
            };
            let tx = TxLegacy {
                chain_id: Some(self.chain_id),
                nonce,
                gas_price: SLASH_GAS_PRICE,
                gas_limit: SLASH_GAS_LIMIT,
                to: TxKind::Call(target),
                value: U256::ZERO,
                input: calldata,
            };
            let sig = match self.signer.sign_hash_sync(&tx.signature_hash()) {
                Ok(s) => s,
                Err(e) => return SubmitOutcome::Failed(format!("sign: {e}")),
            };
            let signed = tx.into_signed(sig);
            let tx_hash: B256 = *signed.hash();
            let envelope: EthereumTxEnvelope<TxEip4844> = EthereumTxEnvelope::Legacy(signed);
            let recovered = Recovered::new_unchecked(envelope, self.signer_address);

            // 3. Submit and subscribe to the tx's event stream.
            let mut events = match self
                .pool
                .add_consensus_transaction_and_subscribe(recovered, TransactionOrigin::Local)
                .await
            {
                Ok(e) => e,
                Err(e) => {
                    warn!(%tx_hash, %target, ?e, "pool rejected slash tx");
                    return SubmitOutcome::Failed(format!("pool add: {e:?}"));
                }
            };
            debug!(%tx_hash, %target, "slash tx submitted; awaiting inclusion");

            // 4. Await on-chain inclusion (bounded — the sim gate already
            //    confirmed the tx would succeed, so this only guards a wedged
            //    pool from blocking the consumer indefinitely).
            let inclusion = tokio::time::timeout(SLASH_INCLUSION_TIMEOUT, async {
                while let Some(ev) = events.next().await {
                    match ev {
                        TransactionEvent::Mined(_) => return Some(true),
                        TransactionEvent::Replaced(_)
                        | TransactionEvent::Discarded
                        | TransactionEvent::Invalid => return Some(false),
                        _ => {}
                    }
                }
                None
            })
            .await;
            match inclusion {
                Ok(Some(true)) => {}
                Ok(Some(false)) => {
                    return SubmitOutcome::Failed(format!(
                        "slash tx {tx_hash} replaced/discarded/invalid before inclusion"
                    ));
                }
                Ok(None) => {
                    return SubmitOutcome::Failed(format!(
                        "slash tx {tx_hash} event stream ended before inclusion"
                    ));
                }
                Err(_) => {
                    return SubmitOutcome::Failed(format!(
                        "slash tx {tx_hash} not mined within {SLASH_INCLUSION_TIMEOUT:?}"
                    ));
                }
            }

            // 5. Confirm success via the receipt status bit.
            //
            // Not reorg-safe but fail-safe: the receipt may be from a block
            // that later reorgs away. If that happens the tombstone the slash
            // wrote also reverts, so a subsequent retry either re-mines the
            // slash or hits the `slashed`-already guard and returns
            // `AlreadySlashed`. Worst case is a redundant resubmission — never
            // a wrong ack of an un-applied slash.
            match self.provider.receipt_by_hash(tx_hash) {
                Ok(Some(receipt)) if receipt.status() => SubmitOutcome::Mined { tx_hash },
                Ok(Some(_)) => SubmitOutcome::Failed(format!(
                    "slash tx {tx_hash} mined but reverted on-chain (status=0)"
                )),
                Ok(None) => SubmitOutcome::Failed(format!(
                    "slash tx {tx_hash} reported mined but receipt not found"
                )),
                Err(e) => SubmitOutcome::Failed(format!("receipt lookup for {tx_hash}: {e:?}")),
            }
        })
    }
}
