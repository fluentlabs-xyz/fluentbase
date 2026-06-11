//! Resolver for the cert-follower's marshal gap-repair.
//!
//! Implements [`commonware_resolver::Resolver`] for the marshal's gap-repair
//! machinery. A `Block` request is served from the **local reth** first
//! (the follower already holds finalized bodies); a `Finalized` request is
//! served by pulling the certificate from the **upstream** WS. Notarized
//! requests are ignored (the follower is finalized-only — F2=b). Mirrors tempo
//! `follow/resolver.rs`, with the upstream behind the [`CertUpstream`] trait so
//! `consensus` stays transport-agnostic.

use super::upstream::CertUpstream;
use crate::digest::Digest;
use bytes::Bytes;
use commonware_codec::Encode as _;
use commonware_consensus::{marshal::resolver::handler, types::Height};
use commonware_cryptography::ed25519::PublicKey;
use commonware_runtime::{spawn_cell, ContextCell, Spawner};
use commonware_utils::{
    channel::{fallible::FallibleExt as _, mpsc},
    futures::{AbortablePool, Aborter},
    vec::NonEmptyVec,
};

use std::collections::BTreeMap;
use tokio::select;
use tracing::{debug, instrument, warn};

pub(crate) fn try_init<TContext, U>(
    context: TContext,
    config: Config<U>,
) -> (
    Resolver<TContext, U>,
    Mailbox,
    mpsc::Receiver<handler::Message<Digest>>,
) {
    let (handler_tx, handler_rx) = mpsc::channel(config.mailbox_size);
    let (mailbox_tx, mailbox_rx) = mpsc::unbounded_channel();
    let actor = Resolver {
        context: ContextCell::new(context),
        config,
        mailbox: mailbox_rx,
        handler_tx,
        requests: BTreeMap::new(),
        fetches: AbortablePool::default(),
    };
    let mailbox = Mailbox { inner: mailbox_tx };
    (actor, mailbox, handler_rx)
}

#[derive(Clone)]
pub(crate) struct Mailbox {
    inner: mpsc::UnboundedSender<Message>,
}

type Predicate<K> = Box<dyn Fn(&K) -> bool + Send>;

enum Message {
    Fetch {
        keys: Vec<handler::Request<Digest>>,
    },
    Cancel {
        key: handler::Request<Digest>,
    },
    Clear,
    Retain {
        predicate: Predicate<handler::Request<Digest>>,
    },
}

pub(crate) struct Config<U> {
    /// For pulling certificates the follower is missing from the upstream.
    pub(super) upstream: U,
    pub(super) mailbox_size: usize,
}

type FetchPool = AbortablePool<(handler::Request<Digest>, Result<Bytes, bool>)>;

pub(crate) struct Resolver<TContext, U> {
    context: ContextCell<TContext>,
    config: Config<U>,
    handler_tx: mpsc::Sender<handler::Message<Digest>>,
    mailbox: mpsc::UnboundedReceiver<Message>,
    requests: BTreeMap<handler::Request<Digest>, Aborter>,
    fetches: FetchPool,
}

impl<TContext, U> Resolver<TContext, U>
where
    TContext: Spawner,
    U: CertUpstream,
{
    async fn run(mut self) {
        loop {
            select!(
                biased;

                response = self.fetches.next_completed() => {
                    if let Ok(resolution) = response {
                        self.handle_fetch_resolution(resolution);
                    }
                }

                Some(msg) = self.mailbox.recv() => {
                    match msg {
                        Message::Fetch { keys } => {
                            for key in keys {
                                self.schedule_request(key);
                            }
                        }
                        Message::Cancel { key } => { self.requests.remove(&key); }
                        Message::Clear => { self.requests.clear(); }
                        Message::Retain { predicate } => {
                            self.requests.retain(move |key, _| predicate(key));
                        }
                    }
                }
            )
        }
    }

    pub(crate) fn start(mut self) -> commonware_runtime::Handle<()> {
        spawn_cell!(self.context, self.run().await)
    }

    #[instrument(skip_all)]
    fn handle_fetch_resolution(
        &mut self,
        (key, resolution): (handler::Request<Digest>, Result<Bytes, bool>),
    ) {
        match resolution {
            Ok(value) => {
                debug!(%key, "fetched value, delivering to marshal");
                self.requests.remove(&key);
                let (response, _) = commonware_utils::channel::oneshot::channel();
                let _ = self.handler_tx.try_send(handler::Message::Deliver {
                    key,
                    value,
                    response,
                });
            }
            Err(true) => {
                debug!(%key, "fetch failed, rescheduling");
                self.requests.remove(&key);
                self.schedule_request(key);
            }
            Err(false) => {
                debug!(%key, "fetch failed, dropping");
                self.requests.remove(&key);
            }
        }
    }

    fn schedule_request(&mut self, key: handler::Request<Digest>) {
        if self.requests.contains_key(&key) {
            debug!(%key, "request already scheduled");
            return;
        }
        let aborter = match &key {
            handler::Request::Block(digest) => {
                // F-type: a Block request keys on the ORDERING digest, which
                // reth cannot resolve (it stores derived EVM blocks) and the
                // upstream serves only by height. The follower's repair path
                // is Finalized-by-height (which carries the body); a bare
                // by-digest body request is dropped.
                debug!(%digest, "dropping by-digest block request (ordering digest; \
                       repair is served by Finalized-by-height)");
                return;
            }
            handler::Request::Finalized { height } => {
                let upstream = self.config.upstream.clone();
                let height = *height;
                let key = key.clone();
                self.fetches
                    .push(async move { (key, resolve_finalized(upstream, height).await) })
            }
            handler::Request::Notarized { .. } => {
                debug!("ignoring request for notarized block (follower is finalized-only)");
                return;
            }
        };
        debug!(%key, "scheduled new request");
        self.requests.insert(key, aborter);
    }
}

/// Serve a `Finalized` request by pulling the certificate from the upstream WS.
#[instrument(skip(upstream), fields(%height))]
async fn resolve_finalized<U: CertUpstream>(upstream: U, height: Height) -> Result<Bytes, bool> {
    let Some(uf) = upstream.get_finalization(height).await else {
        return Err(false);
    };
    Ok((uf.finalization, uf.block).encode())
}

impl commonware_resolver::Resolver for Mailbox {
    type Key = handler::Request<Digest>;
    type PublicKey = PublicKey;

    async fn fetch(&mut self, key: Self::Key) {
        self.fetch_all(vec![key]).await;
    }

    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        self.inner.send_lossy(Message::Fetch { keys });
    }

    async fn fetch_targeted(&mut self, key: Self::Key, _targets: NonEmptyVec<Self::PublicKey>) {
        self.fetch(key).await;
    }

    async fn fetch_all_targeted(
        &mut self,
        requests: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>,
    ) {
        self.fetch_all(requests.into_iter().map(|(k, _)| k).collect())
            .await;
    }

    async fn cancel(&mut self, key: Self::Key) {
        self.inner.send_lossy(Message::Cancel { key });
    }

    async fn clear(&mut self) {
        self.inner.send_lossy(Message::Clear);
    }

    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        self.inner.send_lossy(Message::Retain {
            predicate: Box::new(predicate),
        });
    }
}
