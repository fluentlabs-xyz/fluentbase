//! Body transport for the epoch-key agreement instance.
//!
//! The agreement's `simplex` voter only ever knows a [`DkgProposal`]'s digest;
//! the payload itself is disseminated by a [`buffered::Engine`], the same
//! primitive the ordering plane uses for `OrderBlock` bodies. `buffered` is what
//! makes a parking `verify` cheap: `Mailbox::subscribe(digest)` is a ready-made
//! await hook that resolves the instant the body lands, whether it arrives before
//! or after the vote that names it.
//!
//! No new top-level p2p channel is opened for this. The muxes already in place
//! carry `u64` sub-channels, so the instance takes a slice of that id space that
//! no per-epoch consensus engine can reach — see
//! [`DKG_SUBCHANNEL_BASE`](fluentbase_p2p::constants::DKG_SUBCHANNEL_BASE) for the
//! disjointness argument and for which half of it expires with the `OrderBlock`
//! shrink.

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
    /// re-enter the id space a per-epoch consensus engine registers from. Not
    /// reachable by any real chain (2^32 epochs), but the alias would be silent —
    /// a `register` that succeeds on the WRONG route — so it is refused here
    /// rather than defended downstream.
    #[error("target epoch {0} is not below DKG_SUBCHANNEL_BASE; its sub-channel id would alias a per-epoch consensus engine's")]
    EpochOutOfRange(u64),
    /// The muxer refused the registration. `AlreadyRegistered` reaches the caller
    /// as this error and NOT as a panic: it means some other subsystem already
    /// owns the route, and taking the process down for it would turn a wiring bug
    /// into an outage on a plane whose whole purpose is to survive one.
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
/// context of its own choosing.
///
/// The agreement instance needs exactly that: its engines must be spawned from
/// the supervisor task's context, or an external `abort()` of the supervisor
/// would leave them running with nothing left to stop them
/// ([`crate::beacon::dkg_engine`]).
///
/// # Precondition on `peers`
///
/// `peers` MUST resolve `latest.primary` to a set containing
/// `committee[target_epoch]`. `buffered` retains a received body only when its
/// SENDER is in that set (`CW/broadcast/src/buffered/engine.rs:319-322`), so a
/// provider tracking any other set — the CURRENT committee at a change boundary,
/// say — drops every proposal body on the floor. The failure is silent end to end:
/// nothing logs above `debug`, `verify` parks on a body that is never cached, and
/// the plane simply never converges. The type system cannot carry this — a
/// `Provider` is a runtime view of a mutable peer set, not a committee — so it is
/// stated here and checked nowhere.
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
            // Two agreement proposal bodies retained per PRIMARY sender.
            // Measured, not guessed: the precondition pass counted exactly ONE
            // distinct body per (instance, sender) in every shape it could
            // build — B1, B2/C9 (three mints), B3 (an absent dealer), and a
            // `[0,1] | [2,3]` network cut across six views inside epoch 1's
            // agreement window (`testbed::preconditions::
            // dkg_bodies_per_peer_are_measured_under_a_partition_in_the_agreement_window`);
            // `Plan::Forward` re-sends the SAME digest, which the deque does not
            // grow (`CW:broadcast/src/buffered/engine.rs:331-337`). The second
            // slot is headroom for a re-proposal after a nullify, which the
            // measurement never produced. `MAX_COMMITTEE_SIZE` was 51 bodies of
            // ~154 KiB per sender (R-037).
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

    /// The RECEIVING half of the same contract. `dkg_subchannel` proves an
    /// agreement id can never be minted inside the epoch space; this proves the
    /// ingress classifier can never read one back OUT of it — which is the half
    /// that was missing when a `BASE | E` id reached the frontier as epoch
    /// `BASE | E`.
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
    /// engine's `register(epoch)` for the SAME epoch number, and a second DKG
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
