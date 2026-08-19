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
use fluentbase_consensus::{CertUpstream, UpstreamFinalized};
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
        response: oneshot::Sender<Option<UpstreamFinalized>>,
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
    ) -> impl Future<Output = Option<UpstreamFinalized>> + Send {
        let tx = self.tx.clone();
        async move {
            let (response, rx) = oneshot::channel();
            tx.send(UpstreamMsg::GetFinalizationEverywhere { height, response })
                .ok()?;
            rx.await.ok().flatten()
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
            let client = match WsClientBuilder::default().build(&url).await {
                Ok(c) => {
                    backoff = 1;
                    Arc::new(c)
                }
                Err(e) => {
                    warn!(url = %url, error = %e, backoff, "cert-follow upstream connect failed; rotating");
                    self.ctx.sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
                    continue;
                }
            };
            let mut sub: Subscription<Event> = match client.subscribe_events().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "cert-follow upstream subscribe failed; reconnecting");
                    self.ctx.sleep(Duration::from_secs(1)).await;
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
                        Some(UpstreamMsg::GetFinalization { height, response }) => {
                            // Spawn so an in-flight gap fetch never stalls the live stream.
                            let client = client.clone();
                            drop(self.ctx.with_label("get_finalization").spawn(move |_| async move {
                                let _ = response.send(fetch_finalization(&client, Query::Height(height.get())).await);
                            }));
                        }
                        Some(UpstreamMsg::GetFinalizationEverywhere { height, response }) => {
                            // Same spawn as the plain pull — the walk must never
                            // stall the live stream — plus the URL list and the
                            // actor's own cursor, so it visits every OTHER upstream
                            // and never re-asks the one that just missed.
                            let client = client.clone();
                            let urls = self.urls.clone();
                            let next_url = self.next_url;
                            drop(self.ctx.with_label("get_finalization_everywhere").spawn(move |_| async move {
                                let _ = response.send(
                                    walk_for_height(&client, &urls, next_url, height).await);
                            }));
                        }
                        Some(UpstreamMsg::GetLatest { response }) => {
                            let client = client.clone();
                            drop(self.ctx.with_label("get_latest").spawn(move |_| async move {
                                let _ = response.send(fetch_finalization(&client, Query::Latest).await);
                            }));
                        }
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
                                match msg {
                                    UpstreamMsg::GetFinalization { height, response } => {
                                        let client = client.clone();
                                        drop(self.ctx.with_label("get_finalization").spawn(move |_| async move {
                                            let _ = response.send(
                                                fetch_finalization(&client, Query::Height(height.get())).await);
                                        }));
                                    }
                                    UpstreamMsg::GetLatest { response } => {
                                        let client = client.clone();
                                        drop(self.ctx.with_label("get_latest").spawn(move |_| async move {
                                            let _ = response.send(fetch_finalization(&client, Query::Latest).await);
                                        }));
                                    }
                                    UpstreamMsg::GetFinalizationEverywhere { height, response } => {
                                        let client = client.clone();
                                        let urls = self.urls.clone();
                                        let next_url = self.next_url;
                                        drop(self.ctx.with_label("get_finalization_everywhere").spawn(move |_| async move {
                                            let _ = response.send(
                                                walk_for_height(&client, &urls, next_url, height).await);
                                        }));
                                    }
                                    UpstreamMsg::Rotate { .. } => {
                                        unreachable!("drain_after_rotate kept no Rotate")
                                    }
                                }
                            }
                            break;
                        }
                        None => return, // mailbox dropped → engine gone → shut down
                    },
                }
            }
        }
    }
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
async fn walk_for_height(
    live: &WsClient,
    urls: &[String],
    next_url: usize,
    height: Height,
) -> Option<UpstreamFinalized> {
    match pull(live, Query::Height(height.get())).await {
        Pull::Got(uf) => return Some(*uf),
        Pull::Failed => return None,
        Pull::ContentMiss => {}
    }
    // `next_url` is where the actor would connect NEXT, so starting there and
    // taking `len - 1` visits every OTHER url exactly once and never the live one.
    for offset in 0..urls.len().saturating_sub(1) {
        let url = &urls[(next_url + offset) % urls.len()];
        let client =
            match tokio::time::timeout(WALK_ATTEMPT_TIMEOUT, WsClientBuilder::default().build(url))
                .await
            {
                Ok(Ok(c)) => c,
                _ => continue,
            };
        match pull(&client, Query::Height(height.get())).await {
            Pull::Got(uf) => return Some(*uf),
            Pull::ContentMiss | Pull::Failed => continue,
        }
    }
    // The first place in this system that can say this at all: every configured
    // source was asked and none holds the height. Previously indistinguishable
    // from one slow link.
    warn!(
        height = height.get(),
        upstreams = urls.len(),
        "cert-follow: no configured upstream holds this height"
    );
    None
}

async fn fetch_finalization(client: &WsClient, query: Query) -> Option<UpstreamFinalized> {
    match pull(client, query).await {
        Pull::Got(uf) => Some(*uf),
        Pull::ContentMiss | Pull::Failed => None,
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

    #[derive(Clone)]
    struct Stub {
        behaviour: Behaviour,
        asks: Arc<AtomicUsize>,
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
        async fn subscribe_events(&self, _p: PendingSubscriptionSink) -> SubscriptionResult {
            Ok(())
        }
    }

    async fn serve(behaviour: Behaviour) -> (String, Arc<AtomicUsize>, ServerHandle) {
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

        let got = walk_for_height(&live, &urls, 1, Height::new(7)).await;

        assert!(got.is_none(), "nobody holds it");
        assert_eq!(asks_a.load(Ordering::SeqCst), 1, "the live one, once");
        assert_eq!(asks_b.load(Ordering::SeqCst), 1);
        assert_eq!(asks_c.load(Ordering::SeqCst), 1);
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

        let got = walk_for_height(&live, &urls, 1, Height::new(7)).await;

        assert!(got.is_none());
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
