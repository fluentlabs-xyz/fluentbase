//! Node-side WS upstream actor for `--cert-follow`.
//!
//! Owns one reconnecting jsonrpsee WebSocket connection to an upstream
//! `consensus` RPC. It (1) subscribes to the live finalized stream and pushes
//! each decoded [`UpstreamFinalized`] to the engine's driver, and (2) serves the
//! resolver's by-height [`CertUpstream::get_finalization`] pulls. The hex
//! `CertifiedBlock` is decoded here (`into_parts`), at the crate boundary, so the
//! `consensus` engine never names node RPC types. Mirrors tempo
//! `follow/upstream/actor.rs`, adapted to fluentbase's crate split.

use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use commonware_consensus::types::Height;
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use fluentbase_consensus::{CertUpstream, UpstreamFinalized, WalkOutcome};
use jsonrpsee::{
    core::client::{Error as ClientError, Subscription},
    ws_client::{WsClient, WsClientBuilder},
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::consensus_rpc::{
    server::ConsensusApiClient,
    types::{Event, Query},
};

/// Reconnect backoff ceiling (seconds).
const MAX_BACKOFF_SECS: u64 = 20;

enum UpstreamMsg {
    GetFinalization {
        height: Height,
        response: oneshot::Sender<Option<UpstreamFinalized>>,
    },
    GetLatest {
        response: oneshot::Sender<Option<UpstreamFinalized>>,
    },
    /// By-height pull that, on an explicit CONTENT miss, asks the REST of the
    /// configured upstreams before giving up. Separate from
    /// [`Self::GetFinalization`] on purpose — see [`walk_for_height`].
    GetFinalizationEverywhere {
        height: Height,
        response: oneshot::Sender<WalkOutcome>,
    },
    /// Off-path pull of an epoch-key artifact. Spawned like every other pull so
    /// it can never stall the live subscription, and answered `None` on every
    /// negative alike — see [`CertUpstream::get_epoch_artifact`].
    GetEpochArtifact {
        epoch: u64,
        response: oneshot::Sender<Option<Vec<u8>>>,
    },
    /// Engine-requested rotation: the current upstream served unverifiable
    /// DATA (which a connection-level failover can never detect) — drop the
    /// connection and move to the next URL.
    Rotate { response: oneshot::Sender<()> },
}

/// Cloneable handle the resolver uses for by-height pulls. Implements the
/// consensus-side [`CertUpstream`] seam.
#[derive(Clone)]
pub struct UpstreamHandle {
    tx: mpsc::UnboundedSender<UpstreamMsg>,
}

impl CertUpstream for UpstreamHandle {
    fn get_finalization(
        &self,
        height: Height,
    ) -> impl Future<Output = Option<UpstreamFinalized>> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            tx.send(UpstreamMsg::GetFinalization { height, response })
                .ok()?;
            rx.await.ok().flatten()
        }
    }

    fn get_finalization_everywhere(
        &self,
        height: Height,
    ) -> impl Future<Output = WalkOutcome> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            // A closed mailbox is the actor gone: nothing asked, nothing answered.
            if tx
                .send(UpstreamMsg::GetFinalizationEverywhere { height, response })
                .is_err()
            {
                return WalkOutcome::NoneAnswered;
            }
            rx.await.unwrap_or(WalkOutcome::NoneAnswered)
        }
    }

    fn get_latest(&self) -> impl Future<Output = Option<UpstreamFinalized>> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            tx.send(UpstreamMsg::GetLatest { response }).ok()?;
            rx.await.ok().flatten()
        }
    }

    fn get_epoch_artifact(&self, epoch: u64) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            tx.send(UpstreamMsg::GetEpochArtifact { epoch, response })
                .ok()?;
            rx.await.ok().flatten()
        }
    }

    fn rotate(&self) -> impl Future<Output = ()> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            if tx.send(UpstreamMsg::Rotate { response }).is_ok() {
                let _ = rx.await;
            }
        }
    }
}

/// Bound on the live-finalized stream queued for the driver. BOUNDED (not
/// unbounded) so a malicious/compromised upstream cannot OOM the follower by
/// streaming certs faster than the driver's per-cert BLS verify drains them.
/// On overflow the live event is dropped — the live stream is best-effort; the
/// authoritative path is the marshal's by-height gap-repair (the resolver),
/// which re-pulls any missed height (audit P2-8).
const LIVE_FINALIZED_BUFFER: usize = 256;

/// Build the upstream actor + its by-height pull handle + the live finalized
/// receiver + the connection-generation token. `urls` is the failover list: the
/// actor rotates to the next URL (round-robin) on a connect failure or a dropped
/// connection. The returned [`Arc<AtomicU64>`] is bumped by the actor each time
/// it (re)establishes a connection — the cert-inlet observes it to scope its
/// data-fault streak per-CONNECTION (#7), so a connection-level auto-rotation
/// (which the inlet cannot otherwise see) resets the streak and A's faults never
/// bleed into B's rotation budget.
pub fn init(
    ctx: Context,
    urls: Vec<String>,
) -> (
    UpstreamActor,
    UpstreamHandle,
    mpsc::Receiver<UpstreamFinalized>,
    Arc<AtomicU64>,
) {
    assert!(
        !urls.is_empty(),
        "cert-follow needs at least one upstream URL"
    );
    let (mailbox_tx, mailbox_rx) = mpsc::unbounded_channel();
    let (finalized_tx, finalized_rx) = mpsc::channel(LIVE_FINALIZED_BUFFER);
    let conn_gen = Arc::new(AtomicU64::new(0));
    let actor = UpstreamActor {
        ctx,
        urls,
        next_url: 0,
        mailbox_rx,
        finalized_tx,
        conn_gen: conn_gen.clone(),
    };
    (
        actor,
        UpstreamHandle { tx: mailbox_tx },
        finalized_rx,
        conn_gen,
    )
}

pub struct UpstreamActor {
    ctx: Context,
    urls: Vec<String>,
    next_url: usize,
    mailbox_rx: mpsc::UnboundedReceiver<UpstreamMsg>,
    finalized_tx: mpsc::Sender<UpstreamFinalized>,
    /// Bumped on each successful (re)connect+subscribe so the cert-inlet can
    /// scope its data-fault streak per-CONNECTION (#7). Shared with the inlet via
    /// the [`init`] return.
    conn_gen: Arc<AtomicU64>,
}

impl UpstreamActor {
    pub fn start(self) -> Handle<()> {
        self.ctx
            .clone()
            .with_label("cert_upstream")
            .spawn(move |_| self.run())
    }

    async fn run(mut self) {
        let mut backoff = 1u64;
        loop {
            let url = self.urls[self.next_url % self.urls.len()].clone();
            // Rotate regardless of outcome: a failed connect tries the next
            // URL immediately (with backoff), a dropped connection reconnects
            // to the next one — round-robin failover.
            self.next_url = self.next_url.wrapping_add(1);
            // EVERY await from here to the live `select!` below runs under
            // `while_disconnected`, which is what makes the mailbox served in the
            // "no connection" state instead of silently accumulating (see that
            // function for why silence is not an option).
            let Some(built) = while_disconnected(
                &self.ctx,
                &self.urls,
                self.next_url,
                &mut self.mailbox_rx,
                WsClientBuilder::default().build(&url),
            )
            .await
            else {
                return; // mailbox dropped → engine gone → shut down
            };
            let client = match built {
                Ok(c) => {
                    backoff = 1;
                    Arc::new(c)
                }
                Err(e) => {
                    warn!(url = %url, error = %e, backoff, "cert-follow upstream connect failed; rotating");
                    let slept = while_disconnected(
                        &self.ctx,
                        &self.urls,
                        self.next_url,
                        &mut self.mailbox_rx,
                        self.ctx.sleep(Duration::from_secs(backoff)),
                    )
                    .await;
                    if slept.is_none() {
                        return;
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
                    continue;
                }
            };
            let Some(subscribed) = while_disconnected(
                &self.ctx,
                &self.urls,
                self.next_url,
                &mut self.mailbox_rx,
                client.subscribe_events(),
            )
            .await
            else {
                return;
            };
            let mut sub: Subscription<Event> = match subscribed {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "cert-follow upstream subscribe failed; reconnecting");
                    let slept = while_disconnected(
                        &self.ctx,
                        &self.urls,
                        self.next_url,
                        &mut self.mailbox_rx,
                        self.ctx.sleep(Duration::from_secs(1)),
                    )
                    .await;
                    if slept.is_none() {
                        return;
                    }
                    continue;
                }
            };
            // A new connection is live + serving: bump the generation so the
            // cert-inlet resets its per-connection data-fault streak (#7) — the
            // streak from the prior upstream URL must not count against this one.
            self.conn_gen.fetch_add(1, Ordering::Release);
            debug!(url = %url, "cert-follow upstream connected + subscribed");

            // Serve the live stream + resolver pulls until the connection drops.
            loop {
                tokio::select! {
                    biased;

                    next = sub.next() => match next {
                        Some(Ok(event)) => {
                            if let Some(uf) = decode_finalized(event) {
                                // try_send (never await): a full queue means the
                                // driver is verify-bound; drop the live event and
                                // let gap-repair backfill it (see LIVE_FINALIZED_BUFFER).
                                if self.finalized_tx.try_send(uf).is_err() {
                                    warn!(
                                        "cert-follow live finalized queue full; dropping \
                                         (gap-repair will backfill)"
                                    );
                                }
                            }
                        }
                        Some(Err(e)) => warn!(error = %e, "cert-follow upstream event decode error"),
                        None => {
                            warn!("cert-follow upstream subscription ended; reconnecting");
                            break;
                        }
                    },

                    msg = self.mailbox_rx.recv() => match msg {
                        Some(UpstreamMsg::Rotate { response }) => {
                            warn!(url = %url, "cert-follow: rotating upstream on engine request (data fault)");
                            let _ = response.send(());
                            let (deferred, coalesced) = drain_after_rotate(&mut self.mailbox_rx);
                            if coalesced > 0 {
                                debug!(coalesced, "coalesced concurrent rotate burst into one reconnect");
                            }
                            // The `client` is still alive here (dropped only when the
                            // outer loop iterates), so any pull that interleaved the
                            // rotate burst is served on it — never dropped.
                            for msg in deferred {
                                if dispatch(&self.ctx, &client, &self.urls, self.next_url, msg).is_some() {
                                    unreachable!("drain_after_rotate kept no Rotate")
                                }
                            }
                            break;
                        }
                        Some(msg) => {
                            drop(dispatch(&self.ctx, &client, &self.urls, self.next_url, msg));
                        }
                        None => return, // mailbox dropped → engine gone → shut down
                    },
                }
            }
        }
    }
}

/// Answer one pull on the live connection, spawned so an in-flight fetch never
/// stalls the live stream. The walk additionally takes the URL list and the actor's
/// own cursor, so it visits every OTHER upstream and never re-asks the one that
/// just missed.
///
/// A `Rotate` is not a pull and is handed back untouched: rotation is the actor
/// loop's own decision (it breaks the connection), never a spawned task's.
fn dispatch(
    ctx: &Context,
    client: &Arc<WsClient>,
    urls: &[String],
    next_url: usize,
    msg: UpstreamMsg,
) -> Option<oneshot::Sender<()>> {
    match msg {
        UpstreamMsg::GetFinalization { height, response } => {
            let client = client.clone();
            drop(
                ctx.with_label("get_finalization")
                    .spawn(move |_| async move {
                        let _ = response
                            .send(fetch_finalization(&client, Query::Height(height.get())).await);
                    }),
            );
        }
        UpstreamMsg::GetFinalizationEverywhere { height, response } => {
            let client = client.clone();
            let urls = urls.to_vec();
            drop(
                ctx.with_label("get_finalization_everywhere")
                    .spawn(move |_| async move {
                        let _ = response
                            .send(walk_for_height(Some(&client), &urls, next_url, height).await);
                    }),
            );
        }
        UpstreamMsg::GetLatest { response } => {
            let client = client.clone();
            drop(ctx.with_label("get_latest").spawn(move |_| async move {
                let _ = response.send(fetch_finalization(&client, Query::Latest).await);
            }));
        }
        UpstreamMsg::GetEpochArtifact { epoch, response } => {
            let client = client.clone();
            drop(
                ctx.with_label("get_epoch_artifact")
                    .spawn(move |_| async move {
                        let _ = response.send(fetch_epoch_artifact(&client, epoch).await);
                    }),
            );
        }
        UpstreamMsg::Rotate { response } => return Some(response),
    }
    None
}

/// Drain all immediately-queued mailbox messages after a `Rotate`: ACK + discard every
/// further `Rotate` (coalescing a concurrent inlet+executor rotate burst into ONE
/// reconnect, so `next_url` advances exactly once), and RETURN any interleaved
/// non-Rotate pull to be re-served on the still-live connection. Returns
/// `(deferred_pulls, coalesced_rotate_count)`.
fn drain_after_rotate(rx: &mut mpsc::UnboundedReceiver<UpstreamMsg>) -> (Vec<UpstreamMsg>, usize) {
    let mut deferred = Vec::new();
    let mut coalesced = 0usize;
    while let Ok(msg) = rx.try_recv() {
        match msg {
            UpstreamMsg::Rotate { response } => {
                let _ = response.send(());
                coalesced += 1;
            }
            other => deferred.push(other),
        }
    }
    (deferred, coalesced)
}

/// Answer one mailbox message while the actor has no connection.
///
/// Four of the five get the NEGATIVE their own response type already carries, and
/// none of those negatives is new: a by-height / latest / artifact pull is `None`
/// ("nothing from here — ask again on your own cadence", the answer those callers
/// already handle for a content miss and a transport failure alike), and a `Rotate`
/// is an ACK, because the actor IS between connections and `next_url` has already
/// advanced — the rotation the caller asked for is what is happening.
///
/// [`UpstreamMsg::GetFinalizationEverywhere`] IS THE EXCEPTION, and it is not a
/// special case bolted on: that pull's whole contract is "ask the REST of the
/// configured upstreams", its consumers pay for a miss with an epoch of verify-only
/// or — in crash-survivor recovery — with a verdict of local data loss, and
/// [`walk_for_height`] needs no live connection to honour it (it builds a
/// short-lived one per URL). Refusing it WITHOUT ASKING is what made its negative
/// ambiguous in the first place: a caller cannot tell "every upstream answered and
/// none holds it" from "the actor had no link". So the disconnected actor SERVES it,
/// on the same spawn as the connected path, and the walk names which negative it is
/// (R-131 review, `4.4а-Д-9`).
fn answer_while_disconnected(ctx: &Context, urls: &[String], next_url: usize, msg: UpstreamMsg) {
    match msg {
        UpstreamMsg::GetFinalizationEverywhere { height, response } => {
            let urls = urls.to_vec();
            drop(
                ctx.with_label("get_finalization_everywhere")
                    .spawn(move |_| async move {
                        let _ = response.send(walk_for_height(None, &urls, next_url, height).await);
                    }),
            );
        }
        UpstreamMsg::GetFinalization { response, .. } | UpstreamMsg::GetLatest { response } => {
            let _ = response.send(None);
        }
        UpstreamMsg::GetEpochArtifact { response, .. } => {
            let _ = response.send(None);
        }
        UpstreamMsg::Rotate { response } => {
            let _ = response.send(());
        }
    }
}

/// Drive `fut` — a connect attempt, a backoff sleep, a subscribe attempt: every
/// phase in which this actor has no connection to serve anything on — WHILE
/// reading the mailbox and refusing each message ([`refuse_while_disconnected`]).
/// `None` means the mailbox closed (the engine is gone and the actor must stop);
/// `Some(out)` is `fut`'s own value.
///
/// **A request that cannot be served must be ANSWERED, not left silent, and that
/// is a contract rather than a courtesy.** Every [`UpstreamHandle`] method sends
/// and then awaits a `oneshot` with no deadline of its own — deliberately, since a
/// deadline there would be a timeout on an answer instead of an answer — so a
/// message this actor never reads is a caller that never returns. `.ok()?` on the
/// send catches only a DEAD actor: a live one that is merely not reading its
/// mailbox is indistinguishable from a slow upstream, forever. That is exactly
/// what the outer loop used to do on a failed connect (warn, sleep, `continue`,
/// mailbox untouched), and it cost the `--dpos.follower-upstream`-configured
/// follower below the DPoS activation block its whole entry march: `get_latest`
/// never returned, so the march never re-probed `block_hash(activation)` either
/// (R-131 review, D-01).
async fn while_disconnected<T>(
    ctx: &Context,
    urls: &[String],
    next_url: usize,
    mailbox_rx: &mut mpsc::UnboundedReceiver<UpstreamMsg>,
    fut: impl Future<Output = T>,
) -> Option<T> {
    let mut fut = std::pin::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return Some(out),
            msg = mailbox_rx.recv() => match msg {
                Some(msg) => answer_while_disconnected(ctx, urls, next_url, msg),
                None => return None,
            },
        }
    }
}

/// Decode a live `Event::Finalized` into the engine's [`UpstreamFinalized`].
fn decode_finalized(event: Event) -> Option<UpstreamFinalized> {
    let Event::Finalized { block, .. } = event else {
        // Result-tier events carry no cert+artifact pair; the follower's
        // result view comes from the `result` field of inclusion events.
        return None;
    };
    match block.into_parts() {
        Ok((finalization, block)) => Some(UpstreamFinalized {
            finalization,
            block,
        }),
        Err(e) => {
            warn!(error = %e, "cert-follow: discarding malformed finalized event");
            None
        }
    }
}

/// What a by-height pull came back with. The middle arm is the one that did not
/// exist: an upstream answering "I do not have that height" is not a fault and
/// not a transport failure, it is an honest negative — and it is the only outcome
/// for which asking a DIFFERENT upstream can help.
enum Pull {
    Got(Box<UpstreamFinalized>),
    /// The server answered, explicitly, that it does not hold the height.
    ContentMiss,
    /// Transport, decode, or anything else. Asking elsewhere is not indicated;
    /// connection-level failover already handles the transport case.
    Failed,
}

/// JSON-RPC code the consensus feed answers a by-height miss with
/// (`consensus_rpc::server`). Mirrored here rather than imported because the
/// client half must survive the server half moving.
const NO_CONTENT: i32 = 204;

/// Pull + decode a finalization (`Query::Height` for gap repair, `Query::Latest`
/// for the cold-start EL-sync checkpoint).
async fn pull(client: &WsClient, query: Query) -> Pull {
    match client.get_finalization(query.clone()).await {
        Ok(cb) => match cb.into_parts() {
            Ok((finalization, block)) => Pull::Got(Box::new(UpstreamFinalized {
                finalization,
                block,
            })),
            Err(e) => {
                warn!(error = %e, "cert-follow: malformed getFinalization response");
                Pull::Failed
            }
        },
        Err(ClientError::Call(obj)) if obj.code() == NO_CONTENT => {
            debug!(?query, "cert-follow: upstream does not hold this height");
            Pull::ContentMiss
        }
        Err(e) => {
            debug!(error = %e, ?query, "cert-follow getFinalization failed");
            Pull::Failed
        }
    }
}

/// Per-attempt budget for a walk hop.
///
/// Reused from the consensus plane's own by-height fetch rather than invented:
/// it is the same operation against a different transport, and this file must
/// NOT fall back on `WsClientBuilder::default()`, whose 10 s connect timeout on
/// the boot path costs a measured 2 heights × 10 s per dead URL of startup delay.
const WALK_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);

/// Ask every configured upstream for `height`, in order, until one serves it.
///
/// **One pass, and the bound is the URL list itself** — no new constant. A second
/// lap would ask the same servers the same question: upstream content does not
/// change within one pull, so there is nothing new to learn. Repetition comes
/// from the CALLER's own cadence, which already exists for all three consumers
/// (boot seeding, re-jump landing, crash recovery).
///
/// Only an explicit [`Pull::ContentMiss`] advances the walk. A transport failure
/// stops it: connection-level failover already owns that case, and treating a
/// dead link as "this server lacks the height" would burn the whole list on one
/// broken network.
///
/// Short-lived connections, never the actor's own: the live subscription is
/// untouched, which is the entire reason this is not a rotation. `getFinalization`
/// needs no subscription — the server's feed state is per-node, not per-connection.
///
/// `live == None` is the DISCONNECTED actor serving this pull anyway, and it is
/// the reason the walk can be trusted at all: the alternative — refusing without
/// asking — produces a negative that the caller cannot distinguish from "the
/// record is gone", which is exactly the ambiguity this function exists to remove.
/// With no live link nothing has been asked yet, so the whole list is the walk.
async fn walk_for_height(
    live: Option<&WsClient>,
    urls: &[String],
    next_url: usize,
    height: Height,
) -> WalkOutcome {
    // "Answered" means a server RENDERED A VERDICT on the height — the only thing
    // that licenses `MissedEverywhere`. A connect failure, a timeout and a decode
    // failure are all silence.
    let mut answered = false;
    if let Some(live) = live {
        match pull(live, Query::Height(height.get())).await {
            Pull::Got(uf) => return WalkOutcome::Got(uf),
            // Stops the walk, as before — connection-level failover owns this case.
            // It is NOT an answer, so it cannot end as `MissedEverywhere`.
            Pull::Failed => return WalkOutcome::NoneAnswered,
            Pull::ContentMiss => answered = true,
        }
    }
    // `next_url` is where the actor would connect NEXT, so starting there and
    // taking `len - 1` visits every OTHER url exactly once and never the live one.
    // With no live link there is no "other": every url is unasked.
    let hops = if live.is_some() {
        urls.len().saturating_sub(1)
    } else {
        urls.len()
    };
    for offset in 0..hops {
        let url = &urls[(next_url + offset) % urls.len()];
        let client =
            match tokio::time::timeout(WALK_ATTEMPT_TIMEOUT, WsClientBuilder::default().build(url))
                .await
            {
                Ok(Ok(c)) => c,
                _ => continue,
            };
        match pull(&client, Query::Height(height.get())).await {
            Pull::Got(uf) => return WalkOutcome::Got(uf),
            Pull::ContentMiss => answered = true,
            Pull::Failed => continue,
        }
    }
    if answered {
        // The first place in this system that can say this at all: every configured
        // source was asked and none holds the height. Previously indistinguishable
        // from one slow link.
        warn!(
            height = height.get(),
            upstreams = urls.len(),
            "cert-follow: no configured upstream holds this height"
        );
        WalkOutcome::MissedEverywhere
    } else {
        warn!(
            height = height.get(),
            upstreams = urls.len(),
            "cert-follow: no configured upstream ANSWERED this by-height pull — none of them is \
             reachable right now, which says nothing about whether the height still exists"
        );
        WalkOutcome::NoneAnswered
    }
}

async fn fetch_finalization(client: &WsClient, query: Query) -> Option<UpstreamFinalized> {
    match pull(client, query).await {
        Pull::Got(uf) => Some(*uf),
        Pull::ContentMiss | Pull::Failed => None,
    }
}

/// JSON-RPC code jsonrpsee answers an unknown method with. Nothing versions the
/// `consensus` namespace, so an upstream predating `getEpochArtifact` answers
/// this rather than [`NO_CONTENT`] — and to the caller the two mean the same
/// thing, "no artifact from here". Mapping it to a DATA fault instead would let
/// one old upstream in a failover list rotate a healthy follower off every peer
/// it has.
const METHOD_NOT_FOUND: i32 = -32601;

/// Pull one epoch-key artifact and hex-decode it. Every negative — no content, a
/// method-not-found from an old server, a transport failure, malformed hex — is
/// `None`: the caller's answer to all four is the same (stay unpinned, re-ask on
/// the next certificate), and the bytes are checked against `committee[epoch]`
/// downstream regardless of which server produced them.
async fn fetch_epoch_artifact(client: &WsClient, epoch: u64) -> Option<Vec<u8>> {
    match client.get_epoch_artifact(epoch).await {
        Ok(hex_bytes) => match hex::decode(&hex_bytes) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                warn!(epoch, error = %e, "cert-follow: malformed getEpochArtifact hex");
                None
            }
        },
        Err(ClientError::Call(obj))
            if obj.code() == NO_CONTENT || obj.code() == METHOD_NOT_FOUND =>
        {
            debug!(
                epoch,
                code = obj.code(),
                "cert-follow: upstream serves no artifact for this epoch"
            );
            None
        }
        Err(e) => {
            debug!(epoch, error = %e, "cert-follow getEpochArtifact failed");
            None
        }
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;
    use crate::{
        certified_block::CertifiedBlock,
        consensus_rpc::{server::ConsensusApiServer, types::ConsensusState},
    };
    use jsonrpsee::{
        core::{RpcResult, SubscriptionResult},
        server::{PendingSubscriptionSink, ServerBuilder, ServerHandle},
        types::ErrorObject,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    /// What a stub upstream does with a by-height ask.
    #[derive(Clone, Copy)]
    enum Behaviour {
        /// The honest negative the walk exists to react to.
        NoContent,
        /// A well-formed RPC reply whose payload does not decode — `Pull::Failed`,
        /// which must STOP the walk rather than advance it.
        Malformed,
    }

    /// What a stub upstream does with a `getEpochArtifact` ask. The first two
    /// are the two ways a server says "nothing here", and the whole point is
    /// that the caller cannot tell them apart.
    #[derive(Clone)]
    enum ArtifactAnswer {
        NoContent,
        /// An upstream predating the method. Nothing versions the `consensus`
        /// namespace, so this is what a mixed-version failover list produces.
        MethodNotFound,
        Hex(String),
    }

    #[derive(Clone)]
    struct Stub {
        behaviour: Behaviour,
        asks: Arc<AtomicUsize>,
        artifact: ArtifactAnswer,
    }

    #[jsonrpsee::core::async_trait]
    impl ConsensusApiServer for Stub {
        async fn get_finalization(&self, _q: Query) -> RpcResult<Arc<CertifiedBlock>> {
            self.asks.fetch_add(1, Ordering::SeqCst);
            match self.behaviour {
                Behaviour::NoContent => Err(ErrorObject::owned(
                    NO_CONTENT,
                    "requested finalization not available",
                    None::<()>,
                )),
                Behaviour::Malformed => Ok(Arc::new(CertifiedBlock {
                    height: 1,
                    epoch: 1,
                    view: 1,
                    digest: Default::default(),
                    certificate: "00".into(),
                    block: "00".into(),
                })),
            }
        }
        async fn get_latest(&self) -> RpcResult<ConsensusState> {
            Ok(ConsensusState::default())
        }
        async fn get_epoch_artifact(&self, _epoch: u64) -> RpcResult<String> {
            self.asks.fetch_add(1, Ordering::SeqCst);
            match &self.artifact {
                ArtifactAnswer::NoContent => Err(ErrorObject::owned(
                    NO_CONTENT,
                    "no artifact for that epoch",
                    None::<()>,
                )),
                ArtifactAnswer::MethodNotFound => Err(ErrorObject::owned(
                    METHOD_NOT_FOUND,
                    "Method not found",
                    None::<()>,
                )),
                ArtifactAnswer::Hex(hex) => Ok(hex.clone()),
            }
        }
        async fn subscribe_events(&self, _p: PendingSubscriptionSink) -> SubscriptionResult {
            Ok(())
        }
    }

    async fn serve(behaviour: Behaviour) -> (String, Arc<AtomicUsize>, ServerHandle) {
        serve_artifacts(behaviour, ArtifactAnswer::NoContent).await
    }

    async fn serve_artifacts(
        behaviour: Behaviour,
        artifact: ArtifactAnswer,
    ) -> (String, Arc<AtomicUsize>, ServerHandle) {
        let asks = Arc::new(AtomicUsize::new(0));
        let server = ServerBuilder::default()
            .build("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("ws://{}", server.local_addr().expect("addr"));
        let handle = server.start(
            Stub {
                behaviour,
                asks: asks.clone(),
                artifact,
            }
            .into_rpc(),
        );
        (url, asks, handle)
    }

    /// A CONTENT miss advances the walk, and the walk is exactly ONE pass: every
    /// OTHER configured upstream is asked once, the live one is not re-asked, and
    /// nothing loops.
    ///
    /// Reds if the `NO_CONTENT` arm stops discriminating (the whole thing
    /// collapses to today's behaviour — one ask, give up), and reds if the bound
    /// becomes anything other than the list length.
    #[tokio::test]
    async fn a_content_miss_walks_every_other_upstream_exactly_once() {
        let (url_a, asks_a, _ha) = serve(Behaviour::NoContent).await;
        let (url_b, asks_b, _hb) = serve(Behaviour::NoContent).await;
        let (url_c, asks_c, _hc) = serve(Behaviour::NoContent).await;
        let live = WsClientBuilder::default()
            .build(&url_a)
            .await
            .expect("live");
        let urls = vec![url_a, url_b, url_c];

        let got = walk_for_height(Some(&live), &urls, 1, Height::new(7)).await;

        assert!(
            matches!(got, WalkOutcome::MissedEverywhere),
            "every upstream ANSWERED and none holds it — the one negative that is \
             evidence about the record"
        );
        assert_eq!(asks_a.load(Ordering::SeqCst), 1, "the live one, once");
        assert_eq!(asks_b.load(Ordering::SeqCst), 1);
        assert_eq!(asks_c.load(Ordering::SeqCst), 1);
    }

    /// Nothing versions the `consensus` namespace, so a mixed-version failover
    /// list WILL contain servers that do not know `getEpochArtifact`. Such a
    /// server answers METHOD_NOT_FOUND rather than NO_CONTENT, and the two must
    /// reach the caller identically as "no artifact from here".
    ///
    /// The stake is not cosmetic: this pull shares its mailbox with the by-height
    /// one, whose explicit-miss/failure split drives rotation. If an old server
    /// surfaced here as anything other than a plain negative, one such upstream
    /// would be enough to rotate a healthy follower off every peer it has, over a
    /// method that is optional by design. The third case is what stops the first
    /// two from being vacuous — a client that returned `None` unconditionally
    /// would pass them and fail it.
    #[tokio::test]
    async fn an_old_server_is_a_missing_artifact_and_a_served_one_decodes() {
        let (no_content, asks_nc, _h1) =
            serve_artifacts(Behaviour::NoContent, ArtifactAnswer::NoContent).await;
        let (old, asks_old, _h2) =
            serve_artifacts(Behaviour::NoContent, ArtifactAnswer::MethodNotFound).await;
        let (serving, _asks_s, _h3) =
            serve_artifacts(Behaviour::NoContent, ArtifactAnswer::Hex("00ff10".into())).await;

        let c = WsClientBuilder::default()
            .build(&no_content)
            .await
            .expect("nc");
        assert_eq!(fetch_epoch_artifact(&c, 9).await, None);
        assert_eq!(
            asks_nc.load(Ordering::SeqCst),
            1,
            "the server was really asked"
        );

        let c = WsClientBuilder::default().build(&old).await.expect("old");
        assert_eq!(
            fetch_epoch_artifact(&c, 9).await,
            None,
            "an old server is a missing artifact, never a fault and never bytes"
        );
        assert_eq!(
            asks_old.load(Ordering::SeqCst),
            1,
            "the server was really asked"
        );

        let c = WsClientBuilder::default()
            .build(&serving)
            .await
            .expect("serving");
        assert_eq!(
            fetch_epoch_artifact(&c, 9).await,
            Some(vec![0x00, 0xff, 0x10]),
            "a served artifact is hex-decoded, or the two negatives above prove nothing"
        );
    }

    /// THE TWO NEGATIVES ARE DIFFERENT FACTS, and this is the test that makes them
    /// so. Same height, same list length, same `None` at the mailbox — and the walk
    /// must still separate "every configured upstream answered, none holds it" from
    /// "not one of them answered".
    ///
    /// The stake is irreversible. `dpos::refetch_verified_archive_hole` reads the
    /// first as local consensus data loss and tells the operator to re-sync the EL
    /// disk from a snapshot; the second is a link condition and must never produce
    /// that sentence. Before the entry march made a disconnected actor ANSWER its
    /// mailbox, the second case could not arise (the call simply hung), so nothing
    /// had to tell them apart — the fix for that hang is what made this test
    /// necessary (R-131 review, `4.4а-Д-9`).
    ///
    /// `live = None` is the disconnected actor serving the pull anyway, which is the
    /// other half: a walk that refuses without asking cannot classify anything.
    ///
    /// Reds on any form where the two cases answer the same — fold `NoneAnswered`
    /// into `MissedEverywhere` (or drop the `answered` witness) and the second half
    /// fails naming the URL count it never reached.
    #[tokio::test]
    async fn an_unreachable_list_is_not_a_missing_height() {
        // (1) EVERYBODY ANSWERS, nobody holds it — evidence about the record.
        let (url_a, asks_a, _ha) = serve(Behaviour::NoContent).await;
        let (url_b, asks_b, _hb) = serve(Behaviour::NoContent).await;
        let answering = vec![url_a, url_b];

        let got = walk_for_height(None, &answering, 0, Height::new(7)).await;
        assert!(
            matches!(got, WalkOutcome::MissedEverywhere),
            "two servers rendered a verdict on the height: that IS `MissedEverywhere`"
        );
        assert_eq!(
            (asks_a.load(Ordering::SeqCst), asks_b.load(Ordering::SeqCst)),
            (1, 1),
            "with no live link the whole list is the walk — both were really asked, \
             which is what the verdict rests on"
        );

        // (2) NOBODY ANSWERS. Two ports that were bound and then released, so the
        // addresses are well-formed and nothing is listening — the shape of a dead
        // upstream, not of a malformed URL.
        let (url_c, _asks_c, hc) = serve(Behaviour::NoContent).await;
        let (url_d, _asks_d, hd) = serve(Behaviour::NoContent).await;
        hc.stop().expect("stop c");
        hd.stop().expect("stop d");
        hc.stopped().await;
        hd.stopped().await;
        let silent = vec![url_c, url_d];

        let got = walk_for_height(None, &silent, 0, Height::new(7)).await;
        assert!(
            matches!(got, WalkOutcome::NoneAnswered),
            "not one upstream answered, so NOTHING here is evidence that the height \
             is gone — reporting data loss from this is the irreversible mistake"
        );
    }

    /// A `Failed` pull is NOT a content miss and must not advance the walk.
    /// Treating a broken link as "this server lacks the height" would burn the
    /// whole list on one bad network — and connection-level failover already owns
    /// that case.
    ///
    /// Reds if the malformed/transport arm starts advancing the walk.
    #[tokio::test]
    async fn a_failed_pull_stops_the_walk_instead_of_advancing_it() {
        let (url_a, asks_a, _ha) = serve(Behaviour::Malformed).await;
        let (url_b, asks_b, _hb) = serve(Behaviour::NoContent).await;
        let live = WsClientBuilder::default()
            .build(&url_a)
            .await
            .expect("live");
        let urls = vec![url_a, url_b];

        let got = walk_for_height(Some(&live), &urls, 1, Height::new(7)).await;

        assert!(
            matches!(got, WalkOutcome::NoneAnswered),
            "a decode failure is SILENCE, not a verdict on the height: it must not end \
             as `MissedEverywhere`, which is what licenses a data-loss claim"
        );
        assert_eq!(asks_a.load(Ordering::SeqCst), 1);
        assert_eq!(
            asks_b.load(Ordering::SeqCst),
            0,
            "a decode failure is not an honest negative — do not walk on it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A concurrent inlet+executor rotate burst (two `Rotate` on the SAME mailbox)
    // must coalesce into ONE reconnect (so `next_url` advances exactly once), while
    // an interleaved non-Rotate pull is preserved (re-served on the live connection,
    // never dropped). The full next-url double-advance is integration-only; this
    // unit-tests the Context-free coalescing seam.
    /// A pull that arrives while the actor has NO connection is ANSWERED — a plain
    /// negative for the four pulls, an ACK for the rotate — instead of sitting
    /// silent in a mailbox nobody reads.
    ///
    /// **The assertion is a deadline because the defect is a HANG, not a wrong
    /// value.** On the form this replaced (`warn` + `sleep(backoff)` + `continue`,
    /// mailbox untouched until a connection exists) all five calls await a
    /// `oneshot` nobody will ever send, so there is nothing to compare — only a
    /// caller that never returns. The deadline is paid ONLY on that failure: the
    /// served path arms no timer and completes in the same poll cycle. The
    /// PRODUCTION fix carries no timeout at all — a deadline on a hanging call is
    /// not an answer, it is a guess about one (R-131 review, D-01).
    #[test]
    fn a_disconnected_actor_answers_every_pull_instead_of_going_silent() {
        use commonware_runtime::{tokio::Runner as TokioRunner, Runner as _};
        // The commonware runner rather than `#[tokio::test]`: serving the mailbox
        // while disconnected needs the actor's own `Context` (the `_everywhere` pull
        // is SERVED there, on a spawn, not refused — see `answer_while_disconnected`).
        TokioRunner::default().start(|ctx| async move {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let handle = UpstreamHandle { tx };
            // EMPTY url list: the walk the `_everywhere` pull is served by then has
            // nothing to ask, which is the fastest honest way to reach its negative.
            // Which negative it is, is what `an_unreachable_list_is_not_a_missing_height`
            // pins; here the property is only that the caller gets an ANSWER.
            let urls: Vec<String> = Vec::new();

            let ask = async move {
                let answers = (
                    handle.get_latest().await.is_none(),
                    handle.get_finalization(Height::new(7)).await.is_none(),
                    matches!(
                        handle.get_finalization_everywhere(Height::new(7)).await,
                        WalkOutcome::MissedEverywhere | WalkOutcome::NoneAnswered
                    ),
                    handle.get_epoch_artifact(3).await.is_none(),
                );
                // A `Rotate` that is not ACKed hangs `rotate().await` for exactly the
                // same reason a silent pull hangs `get_latest().await`, so it is asked
                // here rather than trusted to the match arm.
                handle.rotate().await;
                answers
                // `handle` drops HERE, closing the mailbox — which is the other half of
                // the contract and what lets `while_disconnected` return at all.
            };
            // `pending`: the connect attempt / backoff sleep this stands for outlives
            // every one of the five asks. That is the whole condition under test.
            let serve = while_disconnected(&ctx, &urls, 0, &mut rx, std::future::pending::<()>());

            let (answers, stopped) =
                tokio::time::timeout(
                    Duration::from_secs(5),
                    async move { tokio::join!(ask, serve) },
                )
                .await
                .expect(
                    "a disconnected actor must ANSWER its mailbox: a caller still waiting on \
                     a oneshot nobody will send is the D-01 hang",
                );

            assert_eq!(
            answers,
            (true, true, true, true),
            "every pull is answered, and answered NEGATIVE — there is no connection to serve it on"
        );
            assert!(
                stopped.is_none(),
                "the closed mailbox, not the pending future, is what ends the disconnected phase"
            );
        });
    }

    #[test]
    fn drain_after_rotate_coalesces_rotate_burst() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (r1, mut r1_rx) = oneshot::channel();
        let (r2, mut r2_rx) = oneshot::channel();
        let (rg, _rg_rx) = oneshot::channel();
        tx.send(UpstreamMsg::Rotate { response: r1 })
            .expect("send r1");
        tx.send(UpstreamMsg::Rotate { response: r2 })
            .expect("send r2");
        tx.send(UpstreamMsg::GetLatest { response: rg })
            .expect("send pull");

        let (deferred, coalesced) = drain_after_rotate(&mut rx);

        assert_eq!(coalesced, 2, "both Rotates coalesced into one reconnect");
        assert_eq!(
            deferred.len(),
            1,
            "the interleaved pull is deferred, not discarded"
        );
        assert!(
            matches!(deferred[0], UpstreamMsg::GetLatest { .. }),
            "the deferred message is the GetLatest pull"
        );
        // Each coalesced Rotate's caller was still ACKed (so `rotate().await` returns).
        r1_rx.try_recv().expect("r1 acked");
        r2_rx.try_recv().expect("r2 acked");
    }
}
