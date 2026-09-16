//! Convergence tests on a 5-node `commonware_runtime::deterministic` runtime
//! running the real `authenticated::discovery::Network` — the actual
//! `FluentP2P::build` wiring, not a mock (the `commonware_p2p::simulated` module
//! is not used here).
//!
//! Exercises the raw `vote` channel directly: per-epoch demux lives in the
//! consensus `EpochManager`, not in this layer.

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

/// Every peer's raw `vote` channel receives every other peer's identity message
/// within deterministic-time bounds, exercising the real `FluentP2P` wiring and
/// commonware discovery fan-out.
#[test]
fn five_node_convergence() {
    let executor = deterministic::Runner::default();
    executor.start(|context| async move {
        let peers: Vec<PrivateKey> = (0..N).map(|i| peer_key(i as u64)).collect();
        let addresses: Vec<PeerPubkey> = peers.iter().map(|p| p.public_key()).collect();
        let peer_set = fluentbase_staking_reader::TrackedPeers {
            committees: vec![(0, Set::try_from(addresses.clone()).expect("distinct keys"))],
            secondary: Set::default(),
        };
        let bootnode_addr = peer_listen(0);

        let (complete_tx, mut complete_rx) = commonware_utils::channel::mpsc::channel::<()>(N);

        for (i, peer) in peers.iter().enumerate() {
            let peer_ctx = context.with_label(&format!("peer_{i}"));
            let listen = peer_listen(i as u64);

            let bootstrappers: Vec<Bootstrapper<PeerPubkey>> = if i == 0 {
                vec![]
            } else {
                vec![(addresses[0].clone(), Ingress::Socket(bootnode_addr))]
            };

            let cfg = make_config(i as u64, listen, bootstrappers);
            let (p2p, mut handles) = FluentP2P::build(peer_ctx.clone(), cfg);

            handles.oracle.track(0, peer_set.clone()).await;

            let mut vote_s = handles.vote_sender;
            let mut vote_r = handles.vote_receiver;

            let _network_handle = p2p.start();

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

                    let msg = me.encode().to_vec();
                    agent_ctx
                        .with_label("tx")
                        .spawn(move |sender_ctx| async move {
                            loop {
                                let recipients = Recipients::Some(others.clone());
                                let _delivered = vote_s
                                    .send(recipients, msg.clone(), false)
                                    .await
                                    .expect("send");
                                sender_ctx.sleep(Duration::from_millis(100)).await;
                            }
                        });

                    let _ = receiver.await;
                });
        }

        drop(complete_tx); // close to let recv return on agent failure.

        let mut completed = 0;
        while completed < N {
            complete_rx.recv().await.expect("agent completion");
            completed += 1;
        }
        assert_eq!(completed, N, "all peers should converge");

        let metrics = context.encode();
        assert!(
            !metrics.contains("messages_rate_limited_total{"),
            "no rate limiting expected: {metrics}"
        );
    });
}

/// IP poisoning recovery: a malicious peer broadcasts an `Info` for another
/// peer's pubkey with its own IP, so the receiving peer's Ed25519 handshake fails
/// on dial; after `dial_fail_limit` retries the bit-vec flips to "unknown" and
/// gossip re-resolves to the legitimate IP.
///
/// Not implemented: forging an `Info` needs the victim's key, and the realistic
/// version needs `dial_fail_limit` overridable, which is hardcoded to commonware's
/// recommended default.
#[test]
#[ignore = "TODO: forge or simulate IP poisoning scenario"]
fn ip_poisoning_recovers_via_dial_retry() {
    // sketch only
}

/// Clock skew rejection: a peer publishing an `Info` with a timestamp greater than
/// now + `synchrony_bound` (default 5s) makes `InfoVerifier::validate` return
/// `Error::SynchronyBound`, and the Info is dropped.
///
/// Not implemented: `Info` is internal to commonware's tracker actor and
/// auto-signed on connect, so injecting a future timestamp means forking the
/// tracker or writing a raw `Payload::Peers` frame.
#[test]
#[ignore = "TODO: introduce clock-offset between peers"]
fn clock_skew_rejected_by_info_verifier() {
    // sketch only
}

/// Bootnode failure resilience: 4 of 5 bootstrappers offline at startup, and the
/// network still converges via the surviving one.
///
/// Not implemented: needs an unreachable address in the deterministic runtime (a
/// peer that never calls `Network::new` + `start`); commonware's `dial_fail_limit`
/// and randomized dial order should then converge via the survivor.
#[test]
#[ignore = "TODO: model bootnode failure in deterministic runtime"]
fn one_surviving_bootnode_still_converges() {
    // sketch only
}
