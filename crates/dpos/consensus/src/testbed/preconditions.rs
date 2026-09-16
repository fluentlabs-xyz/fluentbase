//! Stand measurements over the production stand: what the marshal archives
//! still serve after re-jumps and floor raises, the `best − ordering_finalized`
//! band, and the DKG body count per peer.
//!
//! Every test here asserts its premise first (the node did jump, the floor did
//! rise, the epochs did pass) and only then the observation; a measurement
//! prints its numbers and asserts nothing about them.

use super::stand::{ArchiveEntry, Committees, Outcome, Stand, StandConfig};
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::constants::DKG_SUBCHANNEL_BASE;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const EPOCH_LEN: u64 = 32;
/// The marshal's processed-height gauge under the stand's label chain, the floor
/// `SetFloor` raises and every ack advances.
const MARSHAL_FLOOR: &str = "outer_marshal_processed_height";

/// The height the network reaches before the consensus-plane cut heals.
///
/// It has to be above `park + gate` for the isolated node to land any re-jump
/// (the gate is `min(JUMP_THRESHOLD, 32)` and the park is `last(2) = 95`), and
/// it has to leave the run enough room for the node to catch up: a node that is
/// still cut sits at `landing − K` for good, because its floor advances again
/// only once it processes the chain instead of jumping over it.
const HEAL_ABOVE: u64 = 4 * EPOCH_LEN + 12;

fn last(epoch: u64) -> u64 {
    (epoch + 1) * EPOCH_LEN - 1
}

/// The 4 → 3 → 4 rotation, node 3 out for epochs 3 and 4.
fn rotate_four_three_four() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }))
}

/// `epoch → members` of the schedule above, for the "every member of
/// `committee(E+1)`" quantifier.
fn members_of(epoch: u64) -> Vec<usize> {
    match epoch {
        3 | 4 => vec![0, 1, 2],
        _ => vec![0, 1, 2, 3],
    }
}

fn entry(out: &Outcome, node: usize, height: u64) -> ArchiveEntry {
    out.archive[node].get(&height).copied().unwrap_or_default()
}

/// The `node × epoch → (cert, block)` table at the epoch terminals, as text.
fn terminal_table(out: &Outcome, epochs: u64) -> String {
    let mut s = String::new();
    for i in 0..out.archive.len() {
        s.push_str(&format!("node{i}:"));
        for e in 0..=epochs {
            let a = entry(out, i, last(e));
            s.push_str(&format!(
                " last({e})={}:{}",
                last(e),
                match (a.finalization, a.block) {
                    (true, true) => "pair",
                    (false, true) => "block-only",
                    (true, false) => "cert-only",
                    (false, false) => "none",
                }
            ));
        }
        s.push('\n');
    }
    s
}

fn holes(out: &Outcome, node: usize) -> (Vec<u64>, Vec<u64>) {
    let missing = out.archive[node]
        .iter()
        .filter(|(_, e)| !e.block)
        .map(|(h, _)| *h)
        .collect();
    let block_only = out.archive[node]
        .iter()
        .filter(|(_, e)| e.block && !e.finalization)
        .map(|(h, _)| *h)
        .collect();
    (missing, block_only)
}

/// `(agreement epoch, sender, distinct bodies)` per node, DKG sub-channels only.
fn dkg_bodies(out: &Outcome) -> Vec<Vec<(u64, PeerPubkey, usize)>> {
    out.bodies
        .iter()
        .map(|b| {
            b.iter()
                .filter(|((sub, _), _)| *sub >= DKG_SUBCHANNEL_BASE)
                .map(|((sub, peer), n)| (*sub - DKG_SUBCHANNEL_BASE, peer.clone(), *n))
                .collect()
        })
        .collect()
}

fn max_bodies_per_peer(out: &Outcome) -> usize {
    dkg_bodies(out)
        .iter()
        .flatten()
        .map(|(_, _, n)| *n)
        .max()
        .unwrap_or(0)
}

fn gap_line(hist: &BTreeMap<u64, u64>) -> String {
    format!("{hist:?}")
}

/// With the archive scan on, node 3 leaves the committee at epoch 3, parks,
/// re-jumps twice on the production gate and comes back. After five full epochs
/// every node, the jumper included, holds the `(finalization, block)` pair at
/// every epoch terminal `last(E)`, read through the marshal-mailbox reads
/// `handle_produce` serves a peer's `Finalized{h}` from (`plane_upstream::serve`).
///
/// The finalized archives are `immutable::Archive`, so the `SetFloor` a re-jump
/// issues reclaims nothing and the by-height reads consult no floor. What the run
/// adds is the premise the code alone cannot give, that the jumper's floor really
/// rose above earlier terminals, and that every height carries its own
/// certificate rather than a descendant's. The lag comes from a consensus-plane
/// cut of node 3 inside epoch 2, because since the epoch key became a fetchable
/// artifact the rotation alone leaves a node following the chain.
#[test]
fn every_node_serves_the_terminal_pair_of_every_passed_epoch_after_re_jumps() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    cfg.archive_scan = true;
    let end = 5 * EPOCH_LEN + 8;
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(2 * EPOCH_LEN + 4)
        .consensus_only()
        .heal_above(HEAL_ABOVE);
    let out = stand.run_until(move |p| p.min_height() >= end, Duration::from_secs(400));
    let epochs_passed = 4u64;
    eprintln!(
        "(Д3/a) heights={:?} jumps={:?} floor={:?}\n{}",
        out.heights,
        out.jump_calls[3]
            .iter()
            .map(|c| (c.from, c.outcome, c.landed.map(|l| l.0)))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|i| out.metric(i, MARSHAL_FLOOR))
            .collect::<Vec<_>>(),
        terminal_table(&out, epochs_passed)
    );
    for i in 0..4 {
        let (missing, block_only) = holes(&out, i);
        eprintln!("(Д3/a) node{i}: missing={missing:?} block-only={block_only:?}");
    }
    eprintln!(
        "(Д4) gap on fcu={:?} on landing={:?}",
        out.head_gap_on_fcu.iter().map(gap_line).collect::<Vec<_>>(),
        out.head_gap_on_landing
            .iter()
            .map(gap_line)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "(Д5) dkg bodies per (epoch, sender) = {:?}; max = {}",
        dkg_bodies(&out),
        max_bodies_per_peer(&out)
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    let landings: Vec<u64> = out.jump_calls[3]
        .iter()
        .filter(|c| c.outcome == "Landed")
        .filter_map(|c| c.landed.map(|l| l.0))
        .collect();
    assert!(
        !landings.is_empty(),
        "node 3 never landed a re-jump: {:?}",
        out.jump_calls[3]
    );
    let floor = out
        .metric(3, MARSHAL_FLOOR)
        .expect("the marshal registers its processed-height gauge");
    assert!(
        floor >= *landings.iter().max().unwrap() as f64,
        "node 3's marshal floor {floor} is below its last landing {landings:?}"
    );
    assert!(
        floor > last(3) as f64,
        "node 3's floor {floor} never passed the terminal of epoch 3"
    );
    // Two epochs past `last(E)` means `start(E + 2)`.
    for i in 0..4 {
        assert!(
            out.heights[i] >= (epochs_passed + 1) * EPOCH_LEN,
            "node {i} at {} is not two epochs past last({}) = {}",
            out.heights[i],
            epochs_passed - 1,
            last(epochs_passed - 1)
        );
    }

    for e in 0..=epochs_passed {
        for i in 0..4 {
            let a = entry(&out, i, last(e));
            let member_next = members_of(e + 1).contains(&i);
            assert!(
                a.finalization && a.block,
                "node {i} (member of committee({}) = {member_next}) does not hold the pair at \
                 last({e}) = {}: {a:?}",
                e + 1,
                last(e)
            );
        }
    }
    for i in 0..4 {
        let (missing, block_only) = holes(&out, i);
        assert!(
            block_only.is_empty(),
            "node {i} holds blocks without their own finalization: {block_only:?}"
        );
        assert!(missing.is_empty(), "node {i} has holes: {missing:?}");
    }
}

/// The same rotation over eight epochs: node 3 leaves the committee at epoch 3,
/// its execution stalls at `last(2) = 95`, and its marshal therefore cannot store
/// above the ordering plane's two-epoch ceiling `last(epoch(95) + 2) = 159`. It
/// climbs out by re-jumping repeatedly, and this test reads that climb off the
/// run.
///
/// A gate at or above `2·interval` wedges an execution-stalled node permanently:
/// the reachable gap is bounded by the ceiling, `tip − fin < 3·interval`, and is
/// only `2·interval` when `fin` sits at an epoch terminal as it does here, so the
/// jump never arms. Production computes `JUMP_THRESHOLD.min(interval)`, which is
/// `≤ interval` for every interval, so it cannot configure such a gate.
///
/// The landing is `target.height − K` and the target is a pair this node already
/// holds, so a jump cannot skip a range it has archived; but the jump also raises
/// the floor to `landing − K = target − 2K`, and a rung served far above the
/// contiguous edge can make the tip non-contiguous, in which case anything
/// `try_repair_gaps` has not pulled by the time the floor moves goes under it for
/// good. `seed_boundary_below_floor` therefore stays necessary. In this fixture
/// both jump targets are contiguous tips reached by the node's own climb, so
/// nothing is skipped.
///
/// The lag comes from a consensus-plane cut of node 3 inside epoch 2, because
/// since the epoch key became a fetchable artifact the rotation alone leaves a
/// node following the chain.
#[test]
fn a_node_more_than_two_epochs_behind_freezes_its_marshal_at_the_ceiling_and_climbs_it_by_jumps_to_its_own_tip(
) {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    cfg.archive_scan = true;
    cfg.marshal_tip_series = true;
    let end = 8 * EPOCH_LEN + 8;
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(2 * EPOCH_LEN + 4)
        .consensus_only()
        .heal_above(HEAL_ABOVE + EPOCH_LEN);
    let out = stand.run_until(
        move |p| p.min_height_of(&[0, 1, 2]) >= end,
        Duration::from_secs(600),
    );
    let epochs_passed = 7u64;
    let (missing, block_only) = holes(&out, 3);
    eprintln!(
        "(Д3/b) heights={:?} jumps={:?} floor={:?} marshal_fin={:?}\n{}(Д3/b) node3: \
         missing={missing:?} block-only={block_only:?}",
        out.heights,
        out.jump_calls[3]
            .iter()
            .map(|c| (
                c.from,
                c.outcome,
                c.consumed.map(|c| c.0),
                c.landed.map(|l| l.0)
            ))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|i| out.metric(i, MARSHAL_FLOOR))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|i| out.metric(i, "outer_marshal_finalized_height"))
            .collect::<Vec<_>>(),
        terminal_table(&out, epochs_passed)
    );
    eprintln!(
        "(Д4) gap on fcu={:?} on landing={:?}",
        out.head_gap_on_fcu.iter().map(gap_line).collect::<Vec<_>>(),
        out.head_gap_on_landing
            .iter()
            .map(gap_line)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "(Д5) dkg bodies per (epoch, sender) = {:?}; max = {}",
        dkg_bodies(&out),
        max_bodies_per_peer(&out)
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    let calls = &out.jump_calls[3];
    let landed: Vec<_> = calls.iter().filter(|c| c.outcome == "Landed").collect();
    assert!(
        landed.len() >= 2,
        "node 3 did not climb out by repeated re-jumps: {calls:?}"
    );
    let first = landed[0];
    assert_eq!(
        first.from,
        last(2),
        "the first jump did not start from the park height"
    );
    let ceiling = last(2 + 2);

    let tips = &out.marshal_tip_series[3];
    assert!(
        !tips.is_empty(),
        "no marshal-tip samples — `StandConfig::marshal_tip_series` was not set"
    );
    assert!(
        first.consumed.expect("a landing consumed a target").0 <= ceiling,
        "node 3's first jump target {:?} is ABOVE the two-epoch ceiling {ceiling} — its \
         marshal stored past the ceiling: tips={tips:?}",
        first.consumed
    );
    for c in &landed {
        let (target, _) = c.consumed.expect("a landing consumed a target");
        let (landing, _) = c.landed.expect("asserted Landed");
        // The ceiling is a function of the cursor the jump started from, and
        // `JumpCall::from` is exactly that cursor — so every landing, not only
        // the first, is checkable against its own two-epoch ceiling.
        let own_ceiling = last(c.from / EPOCH_LEN + 2);
        assert!(
            target <= own_ceiling,
            "a jump from {} targeted {target}, above its own two-epoch ceiling \
             {own_ceiling} — the marshal stored past the ceiling: {c:?}",
            c.from
        );
        assert_eq!(
            landing,
            target - crate::order_block::K,
            "a landing is not `target − K` — the target was not the archive pair at the \
             triggering tip: {c:?}"
        );
        assert!(
            tips.iter().any(|t| *t >= target),
            "node 3 jumped to {target}, a height its own marshal tip never reached: {tips:?}"
        );
    }
    let floor = out
        .metric(3, MARSHAL_FLOOR)
        .expect("the marshal registers its processed-height gauge");
    let highest_landing = landed
        .iter()
        .filter_map(|c| c.landed.map(|l| l.0))
        .max()
        .expect("at least one landing");
    assert!(
        floor >= highest_landing as f64,
        "floor {floor} below the highest landing {highest_landing}"
    );
    assert!(
        floor > last(3) as f64,
        "node 3's floor {floor} never passed the terminal of epoch 3"
    );

    // A hole here would mean a target that was not the archived pair at the
    // triggering tip.
    assert!(
        missing.is_empty(),
        "node 3 has holes — in this fixture every jump target was a contiguous tip, so a \
         hole means the target was not the archive pair at the triggering tip: {missing:?}"
    );
    assert!(
        block_only.is_empty(),
        "node 3 holds blocks without their own finalization: {block_only:?}"
    );

    for e in 0..=epochs_passed {
        for i in 0..4 {
            let a = entry(&out, i, last(e));
            assert!(
                a.finalization && a.block,
                "node {i} does not hold the pair at last({e}) = {}: {a:?}",
                last(e)
            );
        }
    }
    for i in 0..3 {
        let (m, b) = holes(&out, i);
        assert!(
            m.is_empty() && b.is_empty(),
            "member {i}: missing={m:?} block-only={b:?}"
        );
    }
    out.assert_lockstep_except(&[]);
}

/// `best − ordering_finalized` on an honest static stand, at the default link
/// and under a slow, lossy one. Prints the event-driven histograms; asserts only
/// that speculation happened at all (the FCU histogram is non-empty on every
/// node), which is the premise of the number.
#[test]
fn the_head_gap_band_is_measured_on_fcu_events() {
    let run = |label: &str, latency_ms: u64, loss: f64| {
        let mut cfg = StandConfig::honest(4, 1);
        cfg.latency = Duration::from_millis(latency_ms);
        cfg.loss = loss;
        let out = Stand::new(cfg).run_until(|p| p.min_height() >= 40, Duration::from_secs(300));
        eprintln!(
            "(Д4/{label}) latency={latency_ms}ms loss={loss} heights={:?} timed_out={} gap on \
             fcu={:?} per-tick max={:?}",
            out.heights,
            out.timed_out,
            out.head_gap_on_fcu.iter().map(gap_line).collect::<Vec<_>>(),
            out.head_gap_series
                .iter()
                .map(|s| s.iter().max().copied().unwrap_or(0))
                .collect::<Vec<_>>()
        );
        assert!(!out.timed_out, "{label}: heights {:?}", out.heights);
        for i in 0..4 {
            assert!(
                !out.head_gap_on_fcu[i].is_empty(),
                "{label}: node {i} moved its canonical chain on no FCU at all"
            );
        }
        out
    };
    run("fast", 10, 0.0);
    run("slow-lossy", 250, 0.05);
}

/// Distinct agreement bodies per `(epoch, sender)` when the consensus plane is
/// cut for six views in the middle of epoch 1's ceremony (dealing ends at
/// `32 + 12`, the agreement runs after it). Prints the count; asserts the
/// premise — the cut happened inside epoch 1 and the key for epoch 2 was still
/// minted — and nothing about the number.
#[test]
fn dkg_bodies_per_peer_are_measured_under_a_partition_in_the_agreement_window() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand
        .partition(&[0, 1], &[2, 3])
        .after_height(EPOCH_LEN + 14)
        .for_views(6);
    let out = stand.run_until(
        |p| p.min_height() >= 2 * EPOCH_LEN + 4,
        Duration::from_secs(300),
    );
    let part = &out.partitions[0];
    eprintln!(
        "(Д5) heights={:?} cut@{:?} heal@{:?} timed_out={} ceremonies={:?} bodies={:?} max={}",
        out.heights,
        part.heights_at_cut,
        part.heights_at_heal,
        out.timed_out,
        (0..4)
            .map(|i| out.metric(i, "dkg_ceremony_ok_total"))
            .collect::<Vec<_>>(),
        dkg_bodies(&out),
        max_bodies_per_peer(&out)
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(
        part.healed_at > part.cut_at,
        "partition never applied: {part:?}"
    );
    assert!(
        part.heights_at_cut
            .iter()
            .all(|h| *h >= EPOCH_LEN && *h < 2 * EPOCH_LEN),
        "the cut was not inside epoch 1: {part:?}"
    );
    for i in 0..4 {
        assert_eq!(
            out.metric(i, "dkg_ceremony_ok_total"),
            Some(1.0),
            "node {i} did not mint PK_2 across the cut"
        );
    }
}

/// A peer tracked as secondary on the real authenticated transport
/// (`commonware_p2p::authenticated::discovery`, run on the deterministic
/// runtime's in-memory sockets): three peers, every one tracking the same
/// `TrackedPeers { primary: {A, B}, secondary: {C} }` at index 0, B and C
/// bootstrapping to A. Each fact is measured separately: whether C's inbound
/// connection is accepted (A's `Recipients::One(C)` send reports C), whether C
/// receives a `Recipients::All` frame from A and from B, and whether A hears a
/// frame C sends it.
#[test]
fn a_secondary_peer_on_the_authenticated_transport_is_accepted_and_heard() {
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_p2p::{
        authenticated::discovery::{Config, Network},
        Manager as _, Receiver as _, Recipients, Sender as _, TrackedPeers,
    };
    use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner as _, Spawner as _};
    use commonware_utils::{ordered::Set, NZU32};
    use std::{
        collections::BTreeSet,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::{Arc, Mutex},
    };

    const BASE_PORT: u16 = 4100;
    let executor = deterministic::Runner::seeded(1);
    let facts = executor.start(|ctx| async move {
        let keys: Vec<PrivateKey> = (0..3u64).map(PrivateKey::from_seed).collect();
        let pks: Vec<PeerPubkey> = keys.iter().map(|k| k.public_key()).collect();
        let tracked = TrackedPeers::new(
            Set::from_iter_dedup([pks[0].clone(), pks[1].clone()]),
            Set::from_iter_dedup([pks[2].clone()]),
        );
        type Received = Vec<BTreeSet<(PeerPubkey, Vec<u8>)>>;
        let mut senders = Vec::new();
        let received: Arc<Mutex<Received>> = Arc::new(Mutex::new(vec![BTreeSet::new(); 3]));
        for (i, key) in keys.iter().enumerate() {
            let port = BASE_PORT + i as u16;
            let bootstrappers = if i == 0 {
                Vec::new()
            } else {
                vec![(
                    pks[0].clone(),
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), BASE_PORT).into(),
                )]
            };
            let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            let cfg = Config::local(
                key.clone(),
                b"e4-preconditions",
                listen,
                listen,
                bootstrappers,
                1024 * 1024,
            );
            let (mut network, mut oracle) = Network::new(ctx.with_label(&format!("peer{i}")), cfg);
            oracle.track(0, tracked.clone()).await;
            let (sender, mut receiver) =
                network.register(0, commonware_runtime::Quota::per_second(NZU32!(100)), 128);
            network.start();
            senders.push(sender);
            let received = received.clone();
            ctx.with_label(&format!("rx{i}"))
                .spawn(move |_| async move {
                    while let Ok((from, msg)) = receiver.recv().await {
                        received.lock().unwrap()[i].insert((from, msg.as_ref().to_vec()));
                    }
                });
        }
        let payload = |i: usize| vec![b'p', i as u8];
        // Keep sending until the virtual deadline; every fact is read from what
        // arrived and from what the sends reported.
        let mut a_reported_c = false;
        let started = ctx.current();
        while ctx.current().duration_since(started).unwrap() < Duration::from_secs(60) {
            for (i, sender) in senders.iter_mut().enumerate() {
                let _ = sender.send(Recipients::All, payload(i), true).await;
            }
            if let Ok(sent) = senders[0]
                .send(Recipients::One(pks[2].clone()), payload(0), true)
                .await
            {
                a_reported_c |= sent.contains(&pks[2]);
            }
            let got = received.lock().unwrap().clone();
            let c_from_a = got[2].contains(&(pks[0].clone(), payload(0)));
            let c_from_b = got[2].contains(&(pks[1].clone(), payload(1)));
            let a_from_c = got[0].contains(&(pks[2].clone(), payload(2)));
            let b_from_c = got[1].contains(&(pks[2].clone(), payload(2)));
            if a_reported_c && c_from_a && c_from_b && a_from_c && b_from_c {
                break;
            }
            ctx.sleep(Duration::from_millis(500)).await;
        }
        let got = received.lock().unwrap().clone();
        let metrics = ctx.encode();
        let dialed = |from: usize, to: usize| -> bool {
            metrics.lines().any(|l| {
                l.starts_with(&format!("peer{from}_dialer_attempts_total"))
                    && l.contains(&format!("peer=\"{}\"", pks[to]))
            })
        };
        let dials = [
            (0, 1, dialed(0, 1)),
            (0, 2, dialed(0, 2)),
            (1, 0, dialed(1, 0)),
            (1, 2, dialed(1, 2)),
            (2, 0, dialed(2, 0)),
            (2, 1, dialed(2, 1)),
        ];
        (
            dials,
            a_reported_c,
            got[2].contains(&(pks[0].clone(), payload(0))),
            got[2].contains(&(pks[1].clone(), payload(1))),
            got[0].contains(&(pks[2].clone(), payload(2))),
            got[1].contains(&(pks[2].clone(), payload(2))),
            ctx.current().duration_since(started).unwrap(),
            metrics
                .lines()
                .filter(|l| {
                    (l.contains("connections") || l.contains("dial")) && !l.starts_with('#')
                })
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    });
    let (dials, a_reported_c, c_from_a, c_from_b, a_from_c, b_from_c, elapsed, conn_metrics) =
        facts;
    eprintln!(
        "(Д8) A.send(One(C)) reported C={a_reported_c} C<-A={c_from_a} C<-B={c_from_b} \
         A<-C={a_from_c} B<-C={b_from_c} after {elapsed:?}; dial attempts (from, to, \
         attempted)={dials:?}\n{}",
        conn_metrics.join("\n")
    );
    // The secondary dials the primaries and is dialed by nobody; A, the
    // bootstrapper, dials nobody.
    assert_eq!(
        dials,
        [
            (0, 1, false),
            (0, 2, false),
            (1, 0, true),
            (1, 2, false),
            (2, 0, true),
            (2, 1, true)
        ],
        "who dialed whom"
    );
    assert!(
        a_reported_c,
        "A never reported a delivery to the secondary C"
    );
    assert!(
        c_from_a,
        "C (secondary) never received A's Recipients::All frame"
    );
    assert!(
        c_from_b,
        "C (secondary) never received B's Recipients::All frame"
    );
    assert!(a_from_c, "A never heard the secondary C");
    assert!(b_from_c, "B never heard the secondary C");
}
