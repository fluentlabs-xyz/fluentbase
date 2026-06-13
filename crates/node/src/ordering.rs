//! Node-side ordering collaborators for the deferred-execution pipeline:
//! [`ProviderExecutedChain`] (the local derived-chain view) and
//! [`PoolAssembler`] (pool-backed tx selection with the in-flight suffix).

use alloy_primitives::{Address, TxHash, B256};
use fluentbase_consensus::{ExecutedChain, OrderBlock, OrderingAssembler};
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::SignedTransaction as _;
use reth_storage_api::{BlockHashReader, BlockNumReader};
use reth_transaction_pool::{PoolTransaction, TransactionPool};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

/// Provider-backed [`ExecutedChain`]: canonical hash strictly by NUMBER
/// (consistent in-memory + DB view); `last_block_number` as the coarse tip —
/// never `best_number`, whose semantics flip between tree-sync and pipeline
/// backfill.
#[derive(Clone, Debug)]
pub struct ProviderExecutedChain<P>(pub P);

impl<P> ExecutedChain for ProviderExecutedChain<P>
where
    P: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
{
    fn executed_tip(&self) -> u64 {
        self.0.last_block_number().unwrap_or(0)
    }

    fn executed_hash(&self, height: u64) -> Option<B256> {
        self.0.block_hash(height).ok().flatten()
    }
}

/// Per-height slot of the ordered-but-unexecuted suffix: senders' next nonces
/// and included tx hashes from one OrderBlock.
#[derive(Clone, Debug, Default)]
struct HeightSlot {
    next_nonce: HashMap<Address, u64>,
    hashes: HashSet<TxHash>,
}

impl HeightSlot {
    fn from_txs(txs: &[TransactionSigned]) -> Self {
        use alloy_consensus::Transaction as _;
        let mut slot = Self::default();
        for tx in txs {
            slot.hashes.insert(*tx.tx_hash());
            if let Ok(sender) = tx.try_recover() {
                let floor = slot.next_nonce.entry(sender).or_insert(0);
                *floor = (*floor).max(tx.nonce() + 1);
            }
        }
        slot
    }
}

/// The in-flight suffix: per-height slots — finalized entries are
/// authoritative; the proposer's own optimistic stage at a height is REPLACED
/// the moment ANY block finalizes there (own view nullified ⇒ the finalized
/// txs are the truth), so a nullified view's over-high nonce cannot outlive
/// the height it was staged for. Slots at or below the executed tip drop out
/// (the pool's canonical-head tracking covers that range).
#[derive(Debug, Default)]
struct OrderedSuffix {
    finalized: BTreeMap<u64, HeightSlot>,
    staged: BTreeMap<u64, HeightSlot>,
}

impl OrderedSuffix {
    fn prune(&mut self, executed_tip: u64) {
        self.finalized.retain(|h, _| *h > executed_tip);
        self.staged.retain(|h, _| *h > executed_tip);
    }

    fn observe_finalized(&mut self, height: u64, txs: &[TransactionSigned]) {
        self.staged.remove(&height);
        self.finalized.insert(height, HeightSlot::from_txs(txs));
    }

    fn stage(&mut self, height: u64, txs: &[TransactionSigned]) {
        self.staged.insert(height, HeightSlot::from_txs(txs));
    }

    /// Effective filter over all live slots: skip-set of included hashes +
    /// per-sender next-nonce floor (max over slots).
    fn effective(&self) -> (HashSet<TxHash>, HashMap<Address, u64>) {
        let mut hashes = HashSet::new();
        let mut floors: HashMap<Address, u64> = HashMap::new();
        for slot in self.finalized.values().chain(self.staged.values()) {
            hashes.extend(slot.hashes.iter().copied());
            for (sender, next) in &slot.next_nonce {
                let floor = floors.entry(*sender).or_insert(0);
                *floor = (*floor).max(*next);
            }
        }
        (hashes, floors)
    }
}

/// Pool-backed [`OrderingAssembler`]: iterate `best_transactions()`, skip
/// what the suffix says is already ordered, accumulate under the agreed gas
/// limit and the wire byte budget. No execution — admission is the pool's own
/// validation (against the executed head) plus the suffix overlay.
pub struct PoolAssembler<P, XC> {
    pool: P,
    executed: XC,
    suffix: Arc<Mutex<OrderedSuffix>>,
}

impl<P, XC> PoolAssembler<P, XC> {
    pub fn new(pool: P, executed: XC) -> Self {
        Self {
            pool,
            executed,
            suffix: Arc::new(Mutex::new(OrderedSuffix::default())),
        }
    }
}

impl<P, XC> OrderingAssembler for PoolAssembler<P, XC>
where
    P: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>
        + Send
        + Sync
        + 'static,
    XC: ExecutedChain,
{
    fn assemble(&self, height: u64, gas_limit: u64, byte_budget: usize) -> Vec<TransactionSigned> {
        let mut suffix = self.suffix.lock().expect("suffix poisoned");
        suffix.prune(self.executed.executed_tip());
        let (skip_hashes, nonce_floors) = suffix.effective();

        let mut txs: Vec<TransactionSigned> = Vec::new();
        let mut gas_used = 0u64;
        let mut bytes_used = 0usize;
        for pooled in self.pool.best_transactions() {
            if skip_hashes.contains(pooled.hash()) {
                continue;
            }
            if let Some(&floor) = nonce_floors.get(&pooled.sender()) {
                if pooled.nonce() < floor {
                    continue;
                }
            }
            let tx_gas = pooled.gas_limit();
            if gas_used.saturating_add(tx_gas) > gas_limit {
                continue;
            }
            let tx_bytes = pooled.encoded_length();
            if bytes_used.saturating_add(tx_bytes) > byte_budget {
                break;
            }
            gas_used += tx_gas;
            bytes_used += tx_bytes;
            txs.push(pooled.transaction.clone_into_consensus().into_inner());
        }

        suffix.stage(height, &txs);
        txs
    }

    fn observe_finalized(&self, block: &OrderBlock) {
        self.suffix
            .lock()
            .expect("suffix poisoned")
            .observe_finalized(block.height, &block.txs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction as _, TxEip1559};
    use alloy_primitives::{TxKind, U256};
    use alloy_signer::SignerSync as _;
    use alloy_signer_local::PrivateKeySigner;
    use reth_ethereum_primitives::Transaction;

    fn tx(signer: &PrivateKeySigner, nonce: u64) -> TransactionSigned {
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

    // A nullified view's stage must not outlive its height: once ANY block
    // finalizes there, the staged slot is replaced by the finalized txs and
    // the sender's floor reflects what was actually ordered.
    #[test]
    fn finalized_slot_replaces_stage_at_same_height() {
        let signer: PrivateKeySigner =
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                .parse()
                .unwrap();
        let sender = signer.address();
        let mut suffix = OrderedSuffix::default();

        // Own (later nullified) proposal staged nonce 5 at height 10.
        suffix.stage(10, &[tx(&signer, 5)]);
        let (_, floors) = suffix.effective();
        assert_eq!(floors[&sender], 6);

        // A different block finalizes at 10 with nonce 4 only.
        suffix.observe_finalized(10, &[tx(&signer, 4)]);
        let (hashes, floors) = suffix.effective();
        assert_eq!(floors[&sender], 5, "floor follows the FINALIZED txs");
        assert!(!hashes.contains(tx(&signer, 5).tx_hash()));

        // Execution catches up past height 10: the slot drops entirely.
        suffix.prune(10);
        let (hashes, floors) = suffix.effective();
        assert!(hashes.is_empty() && floors.is_empty());
    }
}
