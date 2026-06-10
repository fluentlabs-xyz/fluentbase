//! Cert-follower sync driver.
//!
//! Two inputs, one owner:
//! 1. **Live finalized certs** (`finalized_rx`) pushed by the node's upstream WS
//!    actor. The driver cryptographically verifies the certificate
//!    (`Finalization::verify` against the per-epoch `EpochSchemeProvider`) BEFORE
//!    taking the marshal's *trusted* `verified(round, block)` + `report(Finalization)`
//!    intake — a tampered cert is dropped and never advances the chain. (The
//!    marshal's `report` path itself does NOT re-verify; it is trusted, correct for a
//!    validator whose engine already verified, so the follower must verify here.)
//! 2. **Marshal `Update<Block>`** (the driver is a co-`Reporter` alongside the
//!    executor) → `EpochTransition::on_finalized` for finality-gated epoch
//!    boundary detection. Each boundary surfaces the next epoch's frozen
//!    committee snapshot via `boundary_tx`, which the engine's scheme-forwarder
//!    turns into a BLS verifier and registers (so the marshal can verify the
//!    next epoch's certs). Mirrors tempo `follow/driver.rs`, with fluentbase's
//!    on-chain-committee scheme rotation replacing tempo's DKG-in-extra_data.

use super::{stubs::NullPeerSetSink, upstream::UpstreamFinalized};
use crate::{
    block::Block,
    outer::{EpochSchemeProvider, MarshalMailbox},
    OriginEpocher,
};
use commonware_consensus::{
    marshal::Update, simplex::types::Activity, types::Epocher as _, Heightable as _, Reporter,
};
use commonware_cryptography::{certificate::Provider as _, ed25519, Signer as _};
use commonware_math::algebra::Random as _;
use commonware_parallel::Sequential;
use commonware_runtime::{spawn_cell, tokio::Context, Clock as _, ContextCell, Spawner as _};
use commonware_utils::{vec::NonEmptyVec, Acknowledgement as _};
use fluentbase_staking_reader::{EpochTransition, ReadError, RethStakingStateReader};
use std::time::Duration;
use tokio::{select, sync::mpsc};
use tracing::{error, warn};

/// Executor/new_payload race window: the driver's `Update::Block` clone can beat
/// the executor's `new_payload` on the sibling clone, so the committee read at
/// the block hash transiently sees no state. Retry in place — the block lands
/// within a few ms (same pattern as the validator boundary hook, dpos.rs).
const BLOCK_LANDED_RETRIES: u32 = 100; // ~5s @ 50ms backoff
const BLOCK_LANDED_BACKOFF: Duration = Duration::from_millis(50);

type FollowerEpochTransition<Provider, EvmConfig> =
    EpochTransition<RethStakingStateReader<Provider, EvmConfig>, NullPeerSetSink, Context>;

pub(crate) fn try_init<Provider, EvmConfig>(
    context: Context,
    marshal: MarshalMailbox,
    scheme_provider: EpochSchemeProvider,
    epocher: OriginEpocher,
    finalized_rx: mpsc::Receiver<UpstreamFinalized>,
    epoch_transition: FollowerEpochTransition<Provider, EvmConfig>,
) -> (Driver<Provider, EvmConfig>, MarshalReporter) {
    let (block_tx, block_rx) = mpsc::unbounded_channel();
    let driver = Driver {
        clock: context.clone(),
        context: ContextCell::new(context),
        marshal,
        scheme_provider,
        epocher,
        finalized_rx,
        block_rx,
        epoch_transition,
    };
    (driver, MarshalReporter(block_tx))
}

/// Co-`Reporter` handed to `marshal.start` so the driver sees every finalized
/// `Update<Block>` (for boundary detection) alongside the executor.
#[derive(Clone)]
pub(crate) struct MarshalReporter(mpsc::UnboundedSender<Update<Block>>);

impl Reporter for MarshalReporter {
    type Activity = Update<Block>;

    async fn report(&mut self, activity: Update<Block>) {
        let _ = self.0.send(activity);
    }
}

pub(crate) struct Driver<Provider, EvmConfig> {
    context: ContextCell<Context>,
    clock: Context,
    marshal: MarshalMailbox,
    scheme_provider: EpochSchemeProvider,
    epocher: OriginEpocher,
    finalized_rx: mpsc::Receiver<UpstreamFinalized>,
    block_rx: mpsc::UnboundedReceiver<Update<Block>>,
    epoch_transition: FollowerEpochTransition<Provider, EvmConfig>,
}

impl<Provider, EvmConfig> Driver<Provider, EvmConfig>
where
    Provider: reth_storage_api::StateProviderFactory
        + reth_storage_api::HeaderProvider<Header = alloy_consensus::Header>
        + Send
        + Sync
        + 'static,
    EvmConfig: reth_evm::ConfigureEvm<Primitives = reth_ethereum_primitives::EthPrimitives>
        + Send
        + Sync
        + 'static,
{
    pub(crate) fn start(mut self) -> commonware_runtime::Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    async fn run(mut self) {
        loop {
            select! {
                biased;

                Some(uf) = self.finalized_rx.recv() => self.process_finalized(uf).await,
                Some(update) = self.block_rx.recv() => self.process_block(update).await,
                else => break,
            }
        }
    }

    /// Live path. Two cases by the cert's epoch vs the highest epoch whose committee
    /// scheme is registered (`scheme_provider.highest_epoch()`):
    ///
    /// 1. **Ahead (catch-up):** a cert for a later epoch cannot be verified yet — its
    ///    committee scheme isn't registered until we walk there. Do NOT trust it and
    ///    do NOT drop it: hint the marshal to fetch forward to the highest registered
    ///    epoch's boundary (its `missing_items` path pulls + **verifies** each
    ///    finalization in order, registering the next epoch's scheme as it crosses the
    ///    boundary) and skip. Mirrors tempo `follow/driver.rs`.
    /// 2. **Registered:** the cert's epoch scheme is available — **verify** it
    ///    (`Finalization::verify`) before taking the marshal's *trusted*
    ///    `verified`/`report` intake (which does not re-verify). Drop+log on a
    ///    payload/digest mismatch or a failed multisig check; never finalize an
    ///    unverified cert.
    async fn process_finalized(&mut self, uf: UpstreamFinalized) {
        let round = uf.finalization.proposal.round;
        let epoch = round.epoch();
        let height = uf.block.height().get();

        // Highest epoch we can verify a cert for. The anchor epoch is registered
        // before the driver starts, so this is `Some`; `unwrap_or(epoch)` only guards
        // a never-expected empty provider (treats the cert as registered → no loop).
        let highest = self.scheme_provider.highest_epoch().unwrap_or(epoch);
        if epoch > highest {
            // Drive the marshal's verified forward repair toward the boundary of the
            // highest epoch we *can* verify; crossing it registers the next epoch's
            // scheme, so the next live cert can walk one epoch further. `set_floor`
            // lets the marshal jump rather than re-walk the whole gap. The resolver
            // ignores the target peer, so a fresh dummy key is fine (same as tempo).
            let Some(boundary) = self.epocher.last(highest) else {
                warn!(
                    epoch = epoch.get(),
                    highest = highest.get(),
                    height,
                    "cert-follow: epocher.last overflowed for highest epoch — cannot hint \
                     catch-up boundary (retries on the next finalization)"
                );
                return;
            };
            let dummy = ed25519::PrivateKey::random(&mut self.clock).public_key();
            self.marshal
                .hint_finalized(boundary, NonEmptyVec::new(dummy))
                .await;
            if let Some(before) = boundary.previous() {
                self.marshal.set_floor(before).await;
            }
            return;
        }

        let Some(scheme) = self.scheme_provider.scoped(epoch) else {
            warn!(
                epoch = epoch.get(),
                height,
                "cert-follow: no committee scheme for registered epoch — dropping unverifiable finalization"
            );
            return;
        };

        if uf.finalization.proposal.payload != uf.block.digest() {
            warn!(
                epoch = epoch.get(),
                height,
                "cert-follow: finalization payload != block digest — dropping mismatched cert"
            );
            return;
        }

        if !uf
            .finalization
            .verify(&mut self.clock, scheme.as_ref(), &Sequential)
        {
            warn!(
                epoch = epoch.get(),
                height, "cert-follow: finalization cert FAILED BLS verification — dropping (refusing to finalize)"
            );
            return;
        }

        // Verified → take the (now safe) trusted marshal intake path.
        self.marshal.verified(round, uf.block).await;
        self.marshal
            .report(Activity::Finalization(uf.finalization))
            .await;
    }

    /// Boundary detection: feed the finalized block to `EpochTransition`, which
    /// fires its `boundary_tx` with the next epoch's frozen committee on a
    /// boundary. Retry the committee read while the executor's `new_payload`
    /// catches up (BlockNotFound). Ack the marshal's `Exact` ack afterwards —
    /// this clone of `Update::Block` carries its own ack that must be honoured.
    async fn process_block(&mut self, update: Update<Block>) {
        let Update::Block(block, ack) = update else {
            return; // Tip carries no ack; nothing to rotate.
        };
        let hash = block.block_hash();
        let number = block.height().get();
        let mut retries = 0u32;
        loop {
            match self.epoch_transition.on_finalized(hash, number).await {
                Ok(_) => break,
                Err(ReadError::BlockNotFound(_)) if retries < BLOCK_LANDED_RETRIES => {
                    retries += 1;
                    self.clock.sleep(BLOCK_LANDED_BACKOFF).await;
                }
                Err(e) => {
                    error!(number, ?e, "follower epoch rotation on_finalized failed");
                    break;
                }
            }
        }
        ack.acknowledge();
    }
}
