//! Convergence tests on a 5-node `commonware_runtime::deterministic`
//! runtime running the real `authenticated::discovery::Network` — i.e.
//! tests exercise the actual `FluentP2P::build` wiring, NOT a mock.
//! (File deliberately not named `*_simulated.rs` to avoid confusion
//! with the separate `commonware_p2p::simulated` mock module, which we
//! do NOT use here.)
//!
//! Based on commonware's `p2p discovery/mod.rs` `run_network`, adapted
//! to our `FluentP2P::build` wrapper, exercising the raw `vote` channel
//! directly (no per-epoch Muxer — that demux lives in the consensus
//! EpochManager, not in this layer).

use commonware_codec::Encode as _;
use commonware_cryptography::ed25519::PrivateKey;
use commonware_cryptography::Signer;
use commonware_p2p::{
    authenticated::discovery::Bootstrapper, Ingress, Receiver as _, Recipients, Sender as _,
};
use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner, Spawner as _};
use commonware_utils::ordered::Set;
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use fluentbase_staking_reader::PeerSetSink as _;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const N: usize = 5;
const BASE_PORT: u16 = 9100;

fn peer_key(seed: u64) -> PrivateKey {
    PrivateKey::from_seed(seed)
}

fn peer_listen(seed: u64) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), BASE_PORT + seed as u16)
}

fn make_config(
    seed: u64,
    listen: SocketAddr,
    bootstrappers: Vec<Bootstrapper<PeerPubkey>>,
) -> FluentP2PConfig {
    FluentP2PConfig {
        crypto: peer_key(seed),
        chain_id: 1337,
        listen,
        dialable: Ingress::Socket(listen),
        bootstrappers,
    }
}

/// 5-node convergence: every peer's raw `vote` channel receives every
/// other peer's identity message within deterministic-time bounds.
/// Verifies the real `FluentP2P` wiring + commonware discovery fan-out
/// on the raw vote channel (no Muxer — see module doc).
#[test]
fn five_node_convergence() {
    let executor = deterministic::Runner::default();
    executor.start(|context| async move {
        // Generate N peer keys + listen addresses.
        let peers: Vec<PrivateKey> = (0..N).map(|i| peer_key(i as u64)).collect();
        let addresses: Vec<PeerPubkey> = peers.iter().map(|p| p.public_key()).collect();
        let peer_set: Set<PeerPubkey> = Set::try_from(addresses.clone()).expect("distinct keys");
        let bootnode_addr = peer_listen(0);

        // Build N FluentP2P instances; each tracks the full peer
        // set at index 0 and uses the raw vote channel.
        let (complete_tx, mut complete_rx) = commonware_utils::channel::mpsc::channel::<()>(N);

        for (i, peer) in peers.iter().enumerate() {
            let peer_ctx = context.with_label(&format!("peer_{i}"));
            let listen = peer_listen(i as u64);

            // Peer 0 is the bootnode; peers 1..N dial peer 0.
            let bootstrappers: Vec<Bootstrapper<PeerPubkey>> = if i == 0 {
                vec![]
            } else {
                vec![(addresses[0].clone(), Ingress::Socket(bootnode_addr))]
            };

            let cfg = make_config(i as u64, listen, bootstrappers);
            let (p2p, mut handles) = FluentP2P::build(peer_ctx.clone(), cfg);

            // Track the full peer set at index 0 via the typed PeerSetSink
            // adapter (not the removed `inner_mut` escape hatch).
            handles.oracle.track(0, peer_set.clone()).await;

            // 05 no longer pre-Mux'es vote/cert/resolver (per-epoch demux is
            // owned by 04's EpochManager). For this convergence smoke test
            // we use the raw vote channel directly — no epoch subdivision.
            let mut vote_s = handles.vote_sender;
            let mut vote_r = handles.vote_receiver;

            // Start the network actors (consumes p2p).
            let _network_handle = p2p.start();

            // Per-peer agent: send my pubkey to every other peer; receive
            // (N-1) identity messages and complete.
            let me = peer.public_key();
            let others: Vec<PeerPubkey> = addresses
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, pk)| pk.clone())
                .collect();
            let complete_tx_ = complete_tx.clone();

            peer_ctx
                .with_label("agent")
                .spawn(move |agent_ctx| async move {
                    // Receiver task: collect (N-1) distinct identities, then signal.
                    let tx = complete_tx_;
                    let receiver = agent_ctx.with_label("rx").spawn(move |_| async move {
                        let mut seen = HashSet::new();
                        while seen.len() < N - 1 {
                            let (sender, msg) = vote_r.recv().await.expect("recv");
                            assert_eq!(msg.as_ref(), sender.as_ref(), "msg = sender pubkey");
                            seen.insert(sender);
                        }
                        let _ = tx.send(()).await;
                        // Drain remaining to avoid sender blocking.
                        loop {
                            if vote_r.recv().await.is_err() {
                                break;
                            }
                        }
                    });

                    // Sender task: broadcast my pubkey to all others, retry forever.
                    let msg = me.encode().to_vec();
                    agent_ctx
                        .with_label("tx")
                        .spawn(move |sender_ctx| async move {
                            loop {
                                // Recipients::Some takes ordered Vec; broadcast to others.
                                let recipients = Recipients::Some(others.clone());
                                let _delivered = vote_s
                                    .send(recipients, msg.clone(), false)
                                    .await
                                    .expect("send");
                                sender_ctx.sleep(Duration::from_millis(100)).await;
                            }
                        });

                    // Hold receiver task alive.
                    let _ = receiver.await;
                });
        }

        drop(complete_tx); // close to let recv return on agent failure.

        // Wait until all N peers signal complete.
        let mut completed = 0;
        while completed < N {
            complete_rx.recv().await.expect("agent completion");
            completed += 1;
        }
        assert_eq!(completed, N, "all peers should converge");

        // Sanity: no rate-limiting occurred during the test.
        let metrics = context.encode();
        assert!(
            !metrics.contains("messages_rate_limited_total{"),
            "no rate limiting expected: {metrics}"
        );
    });
}

/// IP poisoning recovery: a malicious peer broadcasts an `Info` for
/// another peer's pubkey with its own IP → the receiving peer's
/// Ed25519 handshake fails on dial → after `dial_fail_limit` retries,
/// the bit-vec flips to "unknown" and gossip re-resolves to the
/// legitimate IP.
///
/// **Implementation deferred** to a follow-up: forging an Info for a
/// peer requires either (a) access to that peer's PrivateKey (defeats
/// the test — handshake would succeed) or (b) crafting a raw
/// `Payload::Peers` message with a different pubkey than the actual
/// dialer, which would be rejected by the `InfoVerifier`
/// signature-check anyway. The realistic version of this test is
/// **two legitimate peers each claiming the same IP** + verifying that
/// `dial_fail_limit` correctly retries — that requires extending
/// `FluentP2PConfig` to override `dial_fail_limit` for the test, which
/// is currently hardcoded to commonware's recommended default (2).
#[test]
#[ignore = "TODO: forge or simulate IP poisoning scenario"]
fn ip_poisoning_recovers_via_dial_retry() {
    // sketch only
}

/// Clock skew rejection: a peer publishes an `Info` with timestamp
/// greater than now + `synchrony_bound` (default 5s) →
/// `InfoVerifier::validate` returns `Error::SynchronyBound` and the
/// Info is dropped.
///
/// **Implementation deferred** to a follow-up: `Info` is internal to
/// commonware's tracker actor (auto-signed on connect from
/// `Config.dialable` + the runtime clock). To inject a future
/// timestamp, we'd need to either fork the tracker or write a raw
/// `Payload::Peers` message — both bypass our `FluentP2P` wrapper.
/// The realistic version of this test is **deterministic clock skew
/// between two peers** (via `deterministic::Runner::with_clock_offset`
/// — pin pending) + observing the late-clock peer's Info rejected.
#[test]
#[ignore = "TODO: introduce clock-offset between peers"]
fn clock_skew_rejected_by_info_verifier() {
    // sketch only
}

/// Bootnode failure resilience: 4 of 5 bootstrappers offline at
/// startup → network still converges via the surviving bootstrapper.
///
/// **Implementation deferred** to a follow-up: requires simulating
/// "this address is unreachable" in the deterministic runtime. The
/// runtime's `Network::dial` impl returns success for any local
/// address that's been bound; making one bind-failure scenario means
/// the corresponding peer simply doesn't `Network::new`+start. Then
/// the other 4 peers list 5 bootstrappers but only 1 has been bound —
/// commonware's `dial_fail_limit` + randomized dial order would
/// converge via the surviving one. Tractable but needs careful
/// setup; deferred for v1.
#[test]
#[ignore = "TODO: model bootnode failure in deterministic runtime"]
fn one_surviving_bootnode_still_converges() {
    // sketch only
}
