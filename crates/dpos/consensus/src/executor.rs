//! Executor: drives the reth EL from ordering-finalized [`OrderBlock`]s —
//! derive → execute (import via `new_payload`) → two-tier FCU.
//!
//! Two-tier forkchoice: `head` follows the locally derived executed tip;
//! `safe`/`finalized` follow RESULT finality = `ordering_finalized − K`
//! (clamped to the cold-start anchor), i.e. the height whose derived hash the
//! committee has attested by agreeing the OrderBlock K heights above it.
//!
//! Ack flow: the marshal's `Exact` ack fires only after derive + import, so
//! marshal backpressure (MAX_PENDING_ACKS) IS execution backpressure.

// **** давай схлопним 3 файла в 1

use crate::{
    application::{BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder, ExecutedChain},
    order_block::OrderBlock,
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::{marshal::Update, types::Height};
use commonware_runtime::{spawn_cell, Clock, ContextCell, FutureExt as _, Handle, Pacer, Spawner};
use commonware_utils::{acknowledgement::Exact, futures::OptionFuture, Acknowledgement as _};
use eyre::{ensure, WrapErr as _};
use futures::{
    future::{ready, BoxFuture, Ready},
    stream::FuturesOrdered,
    FutureExt as _, StreamExt as _,
};
use prometheus_client::metrics::gauge::Gauge;
use std::{ops::RangeInclusive, pin::Pin, time::Duration};
use tokio::{select, sync::mpsc};
use tracing::{debug, error, error_span, info, info_span, instrument, warn, warn_span, Level, Span};

/// One executor command paired with its tracing span (preserves the causal
/// `parent` for `#[instrument]`).
pub struct Message {
    pub cause: Span,
    pub command: Command,
}

pub enum Command {
    /// Derive + import a finalized ordering artifact (`Update::Block`) or
    /// refresh the catch-up target (`Update::Tip`).
    Finalize(Box<Update<OrderBlock>>),
}

pub struct Mailbox {
    tx: mpsc::UnboundedSender<Message>,
}

impl Clone for Mailbox {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl Mailbox {
    fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Test-only constructor used by `application.rs` unit tests to inject a
    /// drain-only mailbox without spawning a real executor.
    #[cfg(test)]
    pub(crate) fn new_for_test(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    /// Sync send — `tokio::sync::mpsc::UnboundedSender::send` never blocks.
    // SendError<Message> carries the rejected message verbatim so the
    // caller can retry; boxing solely to silence the lint would add an
    // alloc on the hot path.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, msg: Message) -> Result<(), mpsc::error::SendError<Message>> {
        self.tx.send(msg)
    }
}

// LastCanonicalized — monotonic projection of forkchoice state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastCanonicalized {
    forkchoice: ForkchoiceState,
    head_height: Height,
    finalized_height: Height,
}

impl LastCanonicalized {
    fn update_finalized(self, height: Height, hash: B256) -> Self {
        let mut this = self;
        if height > this.finalized_height {
            this.finalized_height = height;
            this.forkchoice.safe_block_hash = hash;
            this.forkchoice.finalized_block_hash = hash;
        }
        if height >= this.head_height {
            this.head_height = height;
            this.forkchoice.head_block_hash = hash;
        }
        this
    }

    fn update_head(self, height: Height, hash: B256) -> Self {
        let mut this = self;
        // A lower-height head on the finalized fork (a legitimate reorg of an
        // unfinalized tail — e.g. the migration cold-start where reth's head
        // sits on an orphaned tail) MUST be allowed to roll the head back.
        if height > this.finalized_height || hash == this.forkchoice.finalized_block_hash {
            this.head_height = height;
            this.forkchoice.head_block_hash = hash;
        }
        this
    }
}

// BlockFetcher — minimal trait so we don't depend on the full marshal Mailbox type.

pub trait BlockFetcher: Clone + Send + Sync + 'static {
    fn fetch_block_by_height(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<OrderBlock>> + Send;
}

/// Explicit impl for the concrete marshal mailbox.
/// Orphan rule OK — BlockFetcher local, Mailbox foreign.
impl BlockFetcher
    for commonware_consensus::marshal::core::Mailbox<
        fluentbase_bls::Scheme,
        commonware_consensus::marshal::standard::Standard<OrderBlock>,
    >
{
    async fn fetch_block_by_height(&self, height: Height) -> Option<OrderBlock> {
        self.get_block(height).await
    }
}

pub struct Config<BE, D, XC, MarshalMailbox> {
    pub beacon_engine: BE,
    pub deriver: D,
    pub executed: XC,
    pub marshal: MarshalMailbox,
    pub fcu_heartbeat_interval: Duration,
    pub last_consensus_finalized_height: Height,
    pub last_execution_finalized_height: u64,
    pub initial_finalized: (Height, B256),
    pub initial_head: (Height, B256),
    pub fcu_pace: Duration,
}

pub struct Actor<E, BE, D, XC, MarshalMailbox> {
    context: ContextCell<E>,
    beacon_engine: BE,
    deriver: D,
    executed: XC,
    marshal: MarshalMailbox,
    mailbox: mpsc::UnboundedReceiver<Message>,

    last_canonicalized: LastCanonicalized,
    /// Highest ordering-finalized height processed; drives the result-final
    /// cursor (`− K`, clamped to the anchor).
    ordering_finalized: u64,
    /// Anchor floor for the finalized cursor: the cold-start finalized point
    /// is result-final by construction (committee-external trust root).
    anchor_finalized: (Height, B256),

    fcu_heartbeat_interval: Duration,
    fcu_heartbeat_timer: Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    fcu_pace: Duration,

    finalized_heights_to_backfill: RangeInclusive<u64>,
    pending_backfill: OptionFuture<BoxFuture<'static, (u64, Option<OrderBlock>)>>,
    pending_finalizations: FuturesOrdered<Ready<(Span, OrderBlock, Exact)>>,

    /// Ops-visibility gauge for `pending_finalizations.len()`. Alert on
    /// sustained values > 4 — indicates EL is falling behind consensus
    /// (`MAX_PENDING_ACKS = 16` is the marshal-side ceiling).
    pending_finalizations_gauge: Gauge<i64>,

    /// Heartbeat FCUs are suppressed until consensus advances from the
    /// cold-start snapshot, so a stale initial head is never re-sent over a
    /// canonical chain that moved without us.
    has_advanced_since_init: bool,
}

impl<E, BE, D, XC, MarshalMailbox> Actor<E, BE, D, XC, MarshalMailbox>
where
    E: Clock + commonware_runtime::Metrics + Pacer + Spawner + Send + 'static,
    BE: BeaconEngineLike<
            ExecutionData = D::Derived,
        > + Send
        + Sync
        + 'static,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    MarshalMailbox: BlockFetcher,
{
    pub fn init(context: E, cfg: Config<BE, D, XC, MarshalMailbox>) -> (Self, Mailbox) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mailbox = Mailbox::new(tx);

        let fcu_heartbeat_timer = Box::pin(context.sleep(cfg.fcu_heartbeat_interval));

        let finalized_heights_to_backfill =
            (cfg.last_execution_finalized_height + 1)..=cfg.last_consensus_finalized_height.get();

        let pending_finalizations_gauge = Gauge::<i64>::default();
        context.register(
            "pending_finalizations",
            "Count of finalized blocks awaiting derive+import+ack in the executor queue \
             (MAX_PENDING_ACKS=16 marshal-side ceiling).",
            pending_finalizations_gauge.clone(),
        );

        let actor = Self {
            context: ContextCell::new(context),
            beacon_engine: cfg.beacon_engine,
            deriver: cfg.deriver,
            executed: cfg.executed,
            marshal: cfg.marshal,
            mailbox: rx,
            last_canonicalized: LastCanonicalized {
                forkchoice: ForkchoiceState {
                    head_block_hash: cfg.initial_head.1,
                    safe_block_hash: cfg.initial_finalized.1,
                    finalized_block_hash: cfg.initial_finalized.1,
                },
                head_height: cfg.initial_head.0,
                finalized_height: cfg.initial_finalized.0,
            },
            ordering_finalized: cfg.last_execution_finalized_height,
            anchor_finalized: cfg.initial_finalized,
            fcu_heartbeat_interval: cfg.fcu_heartbeat_interval,
            fcu_heartbeat_timer,
            fcu_pace: cfg.fcu_pace,
            finalized_heights_to_backfill,
            pending_backfill: OptionFuture::default(),
            pending_finalizations: FuturesOrdered::new(),
            pending_finalizations_gauge,
            has_advanced_since_init: false,
        };
        (actor, mailbox)
    }

    pub fn start(mut self) -> Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    async fn run(mut self) {
        info_span!("start").in_scope(|| info!("executor starting"));

        loop {
            if self.pending_backfill.is_none() {
                if let Some(height) = self.finalized_heights_to_backfill.next() {
                    let marshal = self.marshal.clone();
                    self.pending_backfill.replace(
                        async move {
                            (
                                height,
                                marshal.fetch_block_by_height(Height::new(height)).await,
                            )
                        }
                        .boxed(),
                    );
                }
            }

            select! {
                biased;

                (height, maybe_block) = &mut self.pending_backfill => {
                    match maybe_block {
                        Some(block) => {
                            let (ack, _waiter) = Exact::handle();
                            let span = info_span!("backfill_on_start", %height);
                            if let Err(error) = self.forward_finalized(span, block, ack).await {
                                error_span!("shutdown").in_scope(|| error!(%error,
                                    "executor fatal error during backfill; shutting down"));
                                break;
                            }
                        }
                        None => {
                            warn_span!("backfill_on_start", %height).in_scope(||
                                warn!("marshal did not have block in backfill range"));
                        }
                    }
                    // Restore post-completion .is_none() invariant — upstream
                    // OptionFuture does not auto-clear after Poll::Ready, but the
                    // pending_finalizations arm guard below depends on it.
                    *self.pending_backfill = None;
                }

                Some((cause, block, ack)) = self.pending_finalizations.next(),
                if self.pending_backfill.is_none()
                    && self.finalized_heights_to_backfill.is_empty() => {
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                    if let Err(error) = self.forward_finalized(cause, block, ack).await {
                        error_span!("shutdown").in_scope(|| error!(%error,
                            "executor fatal error during finalize; shutting down"));
                        break;
                    }
                }

                msg = self.mailbox.recv() => {
                    let Some(msg) = msg else { break; };
                    self.handle_message(msg).await;
                }

                _ = (&mut self.fcu_heartbeat_timer).fuse() => {
                    self.send_forkchoice_update_heartbeat().await;
                    self.reset_fcu_heartbeat_timer();
                }
            }
        }
    }

    fn reset_fcu_heartbeat_timer(&mut self) {
        self.fcu_heartbeat_timer = Box::pin(self.context.sleep(self.fcu_heartbeat_interval));
    }

    #[instrument(skip_all)]
    async fn send_forkchoice_update_heartbeat(&mut self) {
        if !self.has_advanced_since_init {
            debug!(
                head = %self.last_canonicalized.forkchoice.head_block_hash,
                finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
                "FCU heartbeat suppressed; no consensus advance since cold-start init"
            );
            return;
        }
        info!(
            head = %self.last_canonicalized.forkchoice.head_block_hash,
            finalized = %self.last_canonicalized.forkchoice.finalized_block_hash,
            "FCU heartbeat",
        );
        let resp = self
            .beacon_engine
            .fork_choice_updated(self.last_canonicalized.forkchoice)
            .pace(&self.context, self.fcu_pace)
            .await;
        if let Err(error) = resp {
            warn!(%error, "heartbeat FCU failed");
        }
    }

    async fn handle_message(&mut self, message: Message) {
        let cause = message.cause;
        match message.command {
            Command::Finalize(finalized) => match *finalized {
                // No FCU here: the tip digest is an ORDERING digest reth
                // cannot resolve, and under F-type the EL never needs devp2p
                // for the DPoS segment — catch-up is marshal backfill of
                // OrderBlocks + local derivation, so every derived block's
                // parent is locally present by construction. (A devp2p
                // fast-sync that skips derivation toward an attested `result`
                // hash is a deferred optimization, not a liveness need.)
                Update::Tip(_round, height, _ordering_digest) => {
                    debug!(%height, "ordering tip observed; EL catch-up is backfill+derive");
                }
                Update::Block(block, ack) => {
                    self.pending_finalizations
                        .push_back(ready((cause, block, ack)));
                    self.pending_finalizations_gauge
                        .set(self.pending_finalizations.len() as i64);
                }
            },
        }
    }

    #[instrument(
        skip_all, parent = &cause,
        fields(block.digest = %order.digest(), block.height = %order.height),
        err(level = Level::WARN), ret,
    )]
    async fn forward_finalized(
        &mut self,
        cause: Span,
        order: OrderBlock,
        ack: Exact,
    ) -> eyre::Result<()> {
        let height = order.height;
        let parent_height = height
            .checked_sub(1)
            .ok_or_else(|| eyre::eyre!("ordering height 0 cannot be finalized"))?;
        let parent_hash = match self.executed.executed_hash(parent_height) {
            Some(hash) => hash,
            // The marshal can hold finalized artifacts the EL hasn't derived
            // yet (restart with an unflushed reth tail; repair landing ahead
            // of dispatch). Derivation is strictly sequential, so walk the
            // missing prefix out of the marshal and derive it first; a
            // genuinely unfillable gap stays fatal (visible, not wedged).
            None => self.derive_missing_prefix(parent_height).await?,
        };

        let derived = self
            .deriver
            .derive_and_execute(order, parent_hash)
            .await
            .wrap_err("derive_and_execute failed")?;
        let derived_hash = derived.evm_hash();
        self.submit_finalized_payload(derived).await?;

        self.ordering_finalized = self.ordering_finalized.max(height);
        let result_final = crate::order_block::result_final_height(
            self.ordering_finalized,
            self.anchor_finalized.0.get(),
        );

        let mut new = self.last_canonicalized;
        if result_final > new.finalized_height.get() {
            // The result-final block was derived+FCU'd K iterations ago, so
            // its canonical hash is resolvable; a transient miss keeps the
            // previous finalized cursor (monotonicity over progress).
            match self.executed.executed_hash(result_final) {
                Some(hash) => new = new.update_finalized(Height::new(result_final), hash),
                None => warn!(
                    result_final,
                    "result-final hash unresolved; keeping previous finalized cursor"
                ),
            }
        }
        new = new.update_head(Height::new(height), derived_hash);

        let fcu = self
            .beacon_engine
            .fork_choice_updated(new.forkchoice)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("finalize FCU failed")?;
        ensure!(
            !fcu.is_invalid(),
            "EL reported invalid finalize FCU: {:?}",
            fcu.payload_status
        );
        if new != self.last_canonicalized {
            self.has_advanced_since_init = true;
        }
        self.last_canonicalized = new;
        self.reset_fcu_heartbeat_timer();

        ack.acknowledge();
        Ok(())
    }

    /// Derive the missing `..=target` prefix from the marshal's archive:
    /// probe backward to the highest executed ancestor, then fetch + derive +
    /// import forward. Returns the derived hash AT `target`.
    async fn derive_missing_prefix(&mut self, target: u64) -> eyre::Result<B256> {
        let mut first_missing = target;
        let mut parent_hash = loop {
            if first_missing == 0 {
                return Err(eyre::eyre!("derive gap reaches height 0 — no executed ancestor"));
            }
            if let Some(hash) = self.executed.executed_hash(first_missing - 1) {
                break hash;
            }
            first_missing -= 1;
        };
        info!(
            first_missing,
            target, "deriving missing prefix from marshal before the delivered block"
        );
        for h in first_missing..=target {
            let order = self
                .marshal
                .fetch_block_by_height(Height::new(h))
                .await
                .ok_or_else(|| {
                    eyre::eyre!("derive gap: marshal has no ordering artifact at height {h}")
                })?;
            let derived = self
                .deriver
                .derive_and_execute(order, parent_hash)
                .await
                .wrap_err_with(|| format!("gap derivation failed at height {h}"))?;
            parent_hash = derived.evm_hash();
            self.submit_finalized_payload(derived).await?;
        }
        Ok(parent_hash)
    }

    /// Import the derived block into the EL. VALID is the expected steady
    /// state (single-execution insert acks synthetically; the new_payload
    /// fallback re-executes a block whose parent was derived one iteration
    /// ago); SYNCING is tolerated for the cold-start/rejoin window. Only a
    /// genuinely INVALID status is fatal — under the fallback it means local
    /// derivation diverged from reth's re-execution.
    async fn submit_finalized_payload(&mut self, derived: D::Derived) -> eyre::Result<()> {
        let status = self
            .beacon_engine
            .import_derived(derived)
            .pace(&self.context, self.fcu_pace)
            .await
            .wrap_err("import_derived failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected derived block (local derivation diverged?): `{status:?}`"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order_block::K;
    use crate::digest::Digest;
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
    use alloy_primitives::{Address, Bytes, U256};
    use alloy_rpc_types_engine::{ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use reth_primitives_traits::SealedBlock as RethSealed;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    type RethExecBlock = RethSealed<reth_ethereum_primitives::Block>;

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
        }
    }

    fn sealed_at(parent: B256, number: u64) -> RethExecBlock {
        let header = AlloyHeader {
            parent_hash: parent,
            number,
            gas_limit: 30_000_000,
            timestamp: 1_700_000_000 + number,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        RethSealed::seal_slow(reth_ethereum_primitives::Block::from(AlloyBlock::new(
            header, body,
        )))
    }

    /// Shared height→hash canonical map: the deriver inserts on derive
    /// (modelling new_payload+FCU canonicalization), the ExecutedChain
    /// reads — mirrors the provider-backed production impl.
    #[derive(Clone, Default)]
    struct FakeChain {
        canonical: Arc<Mutex<BTreeMap<u64, B256>>>,
    }

    impl ExecutedChain for FakeChain {
        fn executed_tip(&self) -> u64 {
            self.canonical
                .lock()
                .unwrap()
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
        }
        fn executed_hash(&self, height: u64) -> Option<B256> {
            self.canonical.lock().unwrap().get(&height).copied()
        }
    }

    #[derive(Clone)]
    struct FakeDeriver {
        chain: FakeChain,
    }

    impl DerivedBlockBuilder for FakeDeriver {
        type Derived = RethExecBlock;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
        ) -> eyre::Result<RethExecBlock> {
            let sealed = sealed_at(parent_evm_hash, order.height);
            self.chain
                .canonical
                .lock()
                .unwrap()
                .insert(order.height, sealed.hash());
            Ok(sealed)
        }
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
        new_payload_calls: Arc<Mutex<Vec<RethExecBlock>>>,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> eyre::Result<ForkchoiceUpdated> {
            self.fcu_calls.lock().unwrap().push(state);
            Ok(ForkchoiceUpdated::from_status(PayloadStatusEnum::Valid))
        }

        async fn import_derived(&self, data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            self.new_payload_calls.lock().unwrap().push(data);
            Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
        }
    }

    #[derive(Clone, Default)]
    struct FakeMarshal {
        canned: Arc<Mutex<BTreeMap<u64, OrderBlock>>>,
    }

    impl BlockFetcher for FakeMarshal {
        async fn fetch_block_by_height(&self, height: Height) -> Option<OrderBlock> {
            self.canned.lock().unwrap().get(&height.get()).cloned()
        }
    }

    struct Fixture {
        chain: FakeChain,
        beacon: FakeBeacon,
        marshal: FakeMarshal,
        anchor_hash: B256,
    }

    impl Fixture {
        /// Anchor at `anchor_height` already canonical (reth holds it).
        fn new(anchor_height: u64) -> Self {
            let chain = FakeChain::default();
            let anchor_hash = B256::repeat_byte(0xA0);
            chain
                .canonical
                .lock()
                .unwrap()
                .insert(anchor_height, anchor_hash);
            Self {
                chain,
                beacon: FakeBeacon::default(),
                marshal: FakeMarshal::default(),
                anchor_hash,
            }
        }

        fn build(
            &self,
            ctx: deterministic::Context,
            anchor_height: u64,
            last_consensus: u64,
        ) -> (
            Actor<deterministic::Context, FakeBeacon, FakeDeriver, FakeChain, FakeMarshal>,
            Mailbox,
        ) {
            Actor::init(
                ctx,
                Config {
                    beacon_engine: self.beacon.clone(),
                    deriver: FakeDeriver {
                        chain: self.chain.clone(),
                    },
                    executed: self.chain.clone(),
                    marshal: self.marshal.clone(),
                    fcu_heartbeat_interval: Duration::from_secs(60),
                    last_consensus_finalized_height: Height::new(last_consensus),
                    last_execution_finalized_height: anchor_height,
                    initial_finalized: (Height::new(anchor_height), self.anchor_hash),
                    initial_head: (Height::new(anchor_height), self.anchor_hash),
                    fcu_pace: Duration::from_millis(0),
                },
            )
        }
    }

    fn finalize_msg(order: OrderBlock) -> (Message, commonware_utils::acknowledgement::ExactWaiter) {
        let (ack, waiter) = Exact::handle();
        (
            Message {
                cause: Span::current(),
                command: Command::Finalize(Box::new(Update::Block(order, ack))),
            },
            waiter,
        )
    }

    #[test]
    fn update_head_rolls_back_to_finalized_fork() {
        let anchor = B256::repeat_byte(0x10);
        let tail = B256::repeat_byte(0x12);
        let lc = LastCanonicalized {
            forkchoice: ForkchoiceState {
                head_block_hash: tail,
                safe_block_hash: anchor,
                finalized_block_hash: anchor,
            },
            head_height: Height::new(12),
            finalized_height: Height::new(10),
        };

        let rolled = lc.update_head(Height::new(10), anchor);
        assert_eq!(rolled.head_height, Height::new(10));
        assert_eq!(rolled.forkchoice.head_block_hash, anchor);

        let other = B256::repeat_byte(0x09);
        let unchanged = lc.update_head(Height::new(9), other);
        assert_eq!(unchanged.head_height, Height::new(12));
        assert_eq!(unchanged.forkchoice.head_block_hash, tail);
    }

    // Pre-K window: finalized stays clamped to the anchor while head follows
    // the derived tip; from anchor+K onward finalized = derived hash of −K.
    #[test]
    fn two_tier_finalized_lags_head_by_k_clamped_to_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let mut parent = Digest(B256::ZERO);
            for i in 1..=(K + 1) {
                let height = ANCHOR + i;
                let result = match height.checked_sub(K) {
                    Some(h) if h >= ANCHOR => fx.chain.executed_hash(h).unwrap(),
                    _ => B256::ZERO,
                };
                let order = sample_order(parent, height, result);
                parent = order.digest();
                let (msg, waiter) = finalize_msg(order);
                mailbox.send(msg).expect("send");
                waiter.await.expect("ack");
            }

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                // Heights ANCHOR+1..=ANCHOR+K-1: finalized pinned to the anchor.
                for fcu in &fcus[..(K - 1) as usize] {
                    assert_eq!(fcu.finalized_block_hash, fx.anchor_hash);
                }
                // Height ANCHOR+K: result_final = ANCHOR (still the anchor hash);
                // height ANCHOR+K+1: result_final = ANCHOR+1 = derived hash.
                let derived_anchor_plus_1 = fx.chain.executed_hash(ANCHOR + 1).unwrap();
                let last = fcus.last().unwrap();
                assert_eq!(last.finalized_block_hash, derived_anchor_plus_1);
                assert_eq!(
                    last.head_block_hash,
                    fx.chain.executed_hash(ANCHOR + K + 1).unwrap()
                );
                // Every block was imported exactly once.
                assert_eq!(
                    fx.beacon.new_payload_calls.lock().unwrap().len() as u64,
                    K + 1
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    #[test]
    fn backfill_drains_before_live_finalize() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 0;
            let fx = Fixture::new(ANCHOR);
            // Heights 1..=3 canned in the marshal (crash-recovery backfill).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                let mut parent = Digest(B256::ZERO);
                for h in 1..=3u64 {
                    let order = sample_order(parent, h, B256::ZERO);
                    parent = order.digest();
                    canned.insert(h, order);
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, 3);
            let handle = actor.start();

            // Live finalize for height 4 lands BEFORE backfill drains.
            let parent3 = fx.marshal.canned.lock().unwrap().get(&3).unwrap().digest();
            let (msg, waiter) = finalize_msg(sample_order(parent3, 4, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack for height 4");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(heights, vec![1, 2, 3, 4]);
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // A delivered artifact whose parent is underived (floor jump, unflushed
    // reth tail) must trigger the marshal gap-walk — derive the prefix in
    // order, then the delivered block — instead of a fatal shutdown.
    #[test]
    fn missing_parent_walks_gap_from_marshal() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            // Heights 101..=103 exist ONLY in the marshal (not yet derived).
            {
                let mut canned = fx.marshal.canned.lock().unwrap();
                let mut parent = Digest(B256::ZERO);
                for h in (ANCHOR + 1)..=(ANCHOR + 3) {
                    let order = sample_order(parent, h, B256::ZERO);
                    parent = order.digest();
                    canned.insert(h, order);
                }
            }
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            // Deliver height 104 directly — its parent 103 is underived.
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 4, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack after gap walk");

            {
                let payloads = fx.beacon.new_payload_calls.lock().unwrap();
                let heights: Vec<u64> = payloads.iter().map(|p| p.number).collect();
                assert_eq!(
                    heights,
                    vec![ANCHOR + 1, ANCHOR + 2, ANCHOR + 3, ANCHOR + 4],
                    "gap derived in order before the delivered block"
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }

    // The tip digest is an ordering digest reth cannot resolve — Update::Tip
    // must never become an FCU target.
    #[test]
    fn tip_is_inert_for_forkchoice() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            const ANCHOR: u64 = 100;
            let fx = Fixture::new(ANCHOR);
            let (actor, mailbox) = fx.build(ctx, ANCHOR, ANCHOR);
            let handle = actor.start();

            let tip_digest = Digest(B256::repeat_byte(0xDD));
            mailbox
                .send(Message {
                    cause: Span::current(),
                    command: Command::Finalize(Box::new(Update::Tip(
                        commonware_consensus::types::Round::new(
                            commonware_consensus::types::Epoch::new(0),
                            commonware_consensus::types::View::new(5),
                        ),
                        Height::new(ANCHOR + 50),
                        tip_digest,
                    ))),
                })
                .expect("send tip");

            // Drain barrier: one real finalize.
            let (msg, waiter) =
                finalize_msg(sample_order(Digest(B256::ZERO), ANCHOR + 1, B256::ZERO));
            mailbox.send(msg).expect("send");
            waiter.await.expect("ack");

            {
                let fcus = fx.beacon.fcu_calls.lock().unwrap();
                assert_eq!(fcus.len(), 1, "tip produced no FCU; only the finalize did");
                assert_eq!(
                    fcus[0].head_block_hash,
                    fx.chain.executed_hash(ANCHOR + 1).unwrap()
                );
            }

            drop(mailbox);
            let _ = handle.await;
        });
    }
}
