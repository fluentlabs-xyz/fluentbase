//! Body transport for the epoch-key agreement instance.
//!
//! The agreement's `simplex` voter only knows a [`DkgProposal`]'s digest; the
//! payload itself is disseminated by a [`buffered::Engine`], the same primitive the
//! ordering plane uses for `OrderBlock` bodies. `buffered` is what makes a parking
//! `verify` cheap: `Mailbox::subscribe(digest)` resolves the instant the body lands,
//! whether before or after the vote that names it.
//!
//! No new top-level p2p channel is opened: the muxes already in place carry `u64`
//! sub-channels, so the instance takes a slice of that id space no per-epoch
//! consensus engine can reach.

use commonware_broadcast::buffered;
use commonware_p2p::{
    utils::mux::{Error as MuxError, SubReceiver, SubSender},
    Provider as PeerProvider, Receiver, Sender,
};
use commonware_runtime::{BufferPooler, Clock, Metrics, Spawner};
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::constants::DKG_SUBCHANNEL_BASE;

use crate::{beacon::dkg_agree::DkgProposal, outer::SharedMux};

/// Mailbox depth for the body engine. Sized like the ordering plane's own
/// `buffered` mailbox rather than like a per-committee bound: the traffic is one
/// proposal per member per agreement round, and the engine's own `deque_size`
/// (below) is what bounds retained bodies.
const BODY_MAILBOX_SIZE: usize = 256;

/// The body engine and the mailbox its automaton broadcasts and subscribes on.
pub(crate) type BodyEngine<E, P> = buffered::Engine<E, PeerPubkey, DkgProposal, P>;
pub(crate) type BodyMailbox = buffered::Mailbox<PeerPubkey, DkgProposal>;

/// Why the agreement instance could not take its sub-channel.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TransportError {
    /// The target epoch is large enough that `DKG_SUBCHANNEL_BASE | epoch` would
    /// re-enter the id space a per-epoch consensus engine registers from. Not reachable
    /// by a real chain, but the alias would be silent, so it is refused here.
    #[error("target epoch {0} is not below DKG_SUBCHANNEL_BASE; its sub-channel id would alias a per-epoch consensus engine's")]
    EpochOutOfRange(u64),
    /// The muxer refused the registration. `AlreadyRegistered` reaches the caller as
    /// this error rather than a panic, because some other subsystem owning the route is
    /// a wiring bug, not an outage.
    #[error("dkg sub-channel {subchannel} registration failed: {source}")]
    Register {
        subchannel: u64,
        #[source]
        source: MuxError,
    },
}

/// The sub-channel id the agreement instance for `target_epoch` uses.
///
/// Disjoint from every `register(epoch)` a per-epoch consensus engine issues
/// (`epoch_manager::spawn_engine`) and from the `register(0)` the global
/// singletons issue, because the base is above every representable epoch that
/// passes the range check.
pub(crate) fn dkg_subchannel(target_epoch: u64) -> Result<u64, TransportError> {
    if target_epoch >= DKG_SUBCHANNEL_BASE {
        return Err(TransportError::EpochOutOfRange(target_epoch));
    }
    Ok(DKG_SUBCHANNEL_BASE | target_epoch)
}

/// Register the agreement instance's sub-channel for `target_epoch` on `mux`.
pub(crate) async fn register_dkg_subchannel<HS, HR>(
    mux: &SharedMux<HS, HR>,
    target_epoch: u64,
) -> Result<(SubSender<HS>, SubReceiver<HR>), TransportError>
where
    HS: Sender<PublicKey = PeerPubkey>,
    HR: Receiver<PublicKey = PeerPubkey>,
{
    let subchannel = dkg_subchannel(target_epoch)?;
    mux.lock()
        .await
        .register(subchannel)
        .await
        .map_err(|source| TransportError::Register { subchannel, source })
}

/// Build the body engine without starting it, so the caller can start it on a
/// context of its own choosing: the agreement instance needs its engines spawned
/// from the supervisor task's context, or an external abort of the supervisor would
/// leave them running.
///
/// Precondition on `peers`: it must resolve `latest.primary` to a set containing
/// `committee[target_epoch]`. `buffered` retains a received body only when its
/// sender is in that set, so a provider tracking any other set drops every proposal
/// body silently and the plane never converges.
pub(crate) fn build_body_engine<E, P>(
    context: E,
    me: PeerPubkey,
    peers: P,
) -> (BodyEngine<E, P>, BodyMailbox)
where
    E: BufferPooler + Clock + Spawner + Metrics,
    P: PeerProvider<PublicKey = PeerPubkey>,
{
    buffered::Engine::new(
        context,
        buffered::Config {
            public_key: me,
            mailbox_size: BODY_MAILBOX_SIZE,
            // Two agreement proposal bodies retained per primary sender: a nullify can be
            // followed by a re-proposal from the same leader whose encoding differs, so a peer
            // parked on the first must still find it. A third evicts the first, which is the
            // accepted bound.
            deque_size: 2,
            priority: true,
            codec_config: (),
            peer_provider: peers,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Network},
        utils::mux::Muxer,
    };
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::NZUsize;
    use rand_08::{rngs::StdRng, SeedableRng as _};
    use std::time::Duration;

    /// The production body engine holds two distinct bodies per primary sender and no
    /// more: both are answerable by digest, and a third evicts the first.
    #[test]
    fn two_bodies_from_one_sender_are_both_retained_a_third_evicts_the_first() {
        use crate::beacon::testing::DkgOutcome;
        use commonware_broadcast::Broadcaster as _;
        use commonware_cryptography::bls12381::{
            dkg::deal, primitives::sharing::Mode, primitives::variant::MinSig,
        };
        use commonware_p2p::{Manager as _, Recipients};
        use commonware_utils::{ordered::Set, N3f1};

        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|context| async move {
            let mut rng = StdRng::seed_from_u64(0x53);
            let me = Ed25519PrivateKey::random(&mut rng).public_key();
            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            let channel = oracle
                .control(me.clone())
                .register(
                    fluentbase_p2p::constants::BROADCAST_CHANNEL,
                    fluentbase_p2p::constants::BROADCAST_QUOTA,
                )
                .await
                .expect("register BROADCAST_CHANNEL");
            oracle
                .manager()
                .track(0, Set::from_iter_dedup([me.clone()]))
                .await;
            let (engine, mailbox) =
                build_body_engine(context.with_label("dkg_bodies"), me, oracle.manager());
            drop(engine.start(channel));

            // Three bodies of one target epoch that differ in the pinned set — the
            // way a rebuilt proposal differs from a nullified one.
            let outcome: DkgOutcome = {
                let players: Set<PeerPubkey> = Set::from_iter_dedup(
                    (0..4).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()),
                );
                deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                    .expect("deal")
                    .0
            };
            let body = |pinned: u8| DkgProposal {
                target_epoch: 7,
                logs: (0..=pinned)
                    .map(|i| (i, alloy_primitives::B256::repeat_byte(0x20 + i)))
                    .collect(),
                group_key: outcome.clone(),
                confirms: Vec::new(),
            };
            let (first, second, third) = (body(0), body(1), body(2));
            let digests = [first.digest(), second.digest(), third.digest()];
            assert!(
                digests[0] != digests[1] && digests[1] != digests[2] && digests[0] != digests[2],
                "the fixture's three bodies must be three digests"
            );

            mailbox.broadcast(Recipients::All, first).await;
            mailbox.broadcast(Recipients::All, second).await;
            assert!(
                mailbox.get(digests[0]).await.is_some(),
                "the nullified view's body must still be answerable beside the re-proposal"
            );
            assert!(mailbox.get(digests[1]).await.is_some());

            mailbox.broadcast(Recipients::All, third).await;
            assert!(
                mailbox.get(digests[0]).await.is_none(),
                "a third body from one sender must evict the first — the deque is two deep"
            );
            assert!(mailbox.get(digests[1]).await.is_some());
            assert!(mailbox.get(digests[2]).await.is_some());
        });
    }

    #[test]
    fn subchannel_ids_are_disjoint_from_every_epoch_registration() {
        // The ids a per-epoch consensus engine and the global singletons take.
        for epoch in [0u64, 1, 2, 7, 51, 1_000_000, DKG_SUBCHANNEL_BASE - 1] {
            let sub = dkg_subchannel(epoch).expect("in range");
            assert!(
                sub >= DKG_SUBCHANNEL_BASE,
                "dkg sub-channel {sub} fell into the epoch id space"
            );
            assert_ne!(sub, epoch, "dkg sub-channel aliased register({epoch})");
            assert_ne!(
                sub, 0,
                "dkg sub-channel aliased the singletons' register(0)"
            );
        }
        // Distinct targets never share a route.
        assert_ne!(
            dkg_subchannel(4).expect("in range"),
            dkg_subchannel(5).expect("in range")
        );
    }

    /// The receiving half of the same contract: an agreement sub-channel id can never
    /// be read back out as an epoch.
    #[test]
    fn epoch_from_subchannel_reads_back_the_same_split() {
        use fluentbase_p2p::constants::epoch_from_subchannel;
        for epoch in [0u64, 1, 2, 7, 51, 1_000_000, DKG_SUBCHANNEL_BASE - 1] {
            let sub = dkg_subchannel(epoch).expect("in range");
            assert_eq!(
                epoch_from_subchannel(sub),
                None,
                "agreement sub-channel {sub} was read back as an epoch"
            );
            assert_eq!(
                epoch_from_subchannel(epoch),
                Some(epoch),
                "a per-epoch engine's own registration must stay an epoch"
            );
        }
    }

    #[test]
    fn subchannel_refuses_an_epoch_that_would_alias() {
        let err = dkg_subchannel(DKG_SUBCHANNEL_BASE).expect_err("aliasing epoch");
        assert!(matches!(err, TransportError::EpochOutOfRange(_)), "{err:?}");
    }

    /// A live muxer: the DKG registration must succeed alongside a per-epoch
    /// engine's `register(epoch)` for the same epoch number, and a second DKG
    /// registration for that epoch must come back as `AlreadyRegistered` — an
    /// error the caller can act on, not a panic.
    #[test]
    fn registration_coexists_with_register_epoch_and_reports_a_duplicate() {
        let runner = deterministic::Runner::timed(Duration::from_secs(10));
        runner.start(|context| async move {
            let mut rng = StdRng::seed_from_u64(9);
            let me = Ed25519PrivateKey::random(&mut rng).public_key();

            let (network, oracle) = Network::new(
                context.with_label("network"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(4),
                },
            );
            network.start();
            let (sender, receiver) = oracle
                .control(me.clone())
                .register(
                    fluentbase_p2p::constants::VOTE_CHANNEL,
                    fluentbase_p2p::constants::VOTE_QUOTA,
                )
                .await
                .expect("register VOTE_CHANNEL");

            let (muxer, handle) = Muxer::new(
                context.with_label("mux"),
                sender,
                receiver,
                NZUsize!(16).get(),
            );
            muxer.start();
            let mux: SharedMux<_, _> = std::sync::Arc::new(tokio::sync::Mutex::new(handle));

            const EPOCH: u64 = 12;
            let _engine_route = mux
                .lock()
                .await
                .register(EPOCH)
                .await
                .expect("per-epoch engine route");
            let _dkg_route = register_dkg_subchannel(&mux, EPOCH)
                .await
                .expect("dkg route coexists with the engine route");

            let err = register_dkg_subchannel(&mux, EPOCH)
                .await
                .expect_err("second dkg registration");
            assert!(
                matches!(
                    err,
                    TransportError::Register {
                        source: MuxError::AlreadyRegistered(_),
                        ..
                    }
                ),
                "{err:?}"
            );
        });
    }
}
