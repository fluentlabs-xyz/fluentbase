//! Stand measurements behind the Э4 preconditions pass
//! (`.dpos-study/history/E4-PRECONDITIONS.md`): what the marshal archives
//! still serve after re-jumps and floor raises (Д3), the
//! `best − ordering_finalized` band (Д4), and the DKG body count per peer (Д5).
//!
//! Every test here asserts its PREMISE first — the node did jump, the floor did
//! rise, the epochs did pass — and only then the observation; a measurement
//! prints its numbers and asserts nothing about them.

use super::stand::{ArchiveEntry, Committees, Outcome, Stand, StandConfig};
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::constants::DKG_SUBCHANNEL_BASE;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const EPOCH_LEN: u64 = 32;
/// The marshal's `last_processed_height` gauge (CW `marshal/core/actor.rs:326-332`)
/// under the stand's label chain — the floor `SetFloor` raises and every ack
/// advances.
const MARSHAL_FLOOR: &str = "outer_marshal_processed_height";

fn last(epoch: u64) -> u64 {
    (epoch + 1) * EPOCH_LEN - 1
}

/// The (B2)/(C9) schedule: 4 → 3 → 4, node 3 out for epochs 3 and 4.
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

/// (Д3, gate at the production value) The (C9) fixture with the archive scan on:
/// node 3 leaves the committee at epoch 3, parks, re-jumps twice on the
/// production gate and comes back. After five full epochs EVERY node — the
/// jumper included — holds the `(finalization, block)` pair at every epoch
/// terminal `last(E)`, read through the two marshal-mailbox reads
/// `handle_produce` serves a peer's `Finalized{h}` from (CW
/// `marshal/core/actor.rs:808-818`; `plane_upstream::serve`).
///
/// By the code the result is what it has to be: fluentbase's finalized archives
/// are `immutable::Archive` (`outer.rs:422-423`), whose `prune` is a no-op (CW
/// `marshal/store.rs:223-226`, `:261-264`), so the `SetFloor` a re-jump issues
/// (`executor.rs:2570`) reclaims nothing, and the by-height reads consult no
/// floor (`actor.rs:1369-1393`). What the run adds is the premise the code alone
/// cannot give — that the jumper's floor really rose above earlier terminals —
/// and the answer to (б): every height, terminal or not, carries its OWN
/// certificate (no block-only entries), so `Finalized{last(E)}` is answerable
/// from an explicit finalization rather than a descendant's.
///
/// Falsifier: node 3 not jumping (`jump_calls[3]` empty — the fixture changed);
/// its floor gauge below its first landing (the floor did not rise); a node
/// short of five epochs; a terminal without its pair on a member of the NEXT
/// epoch's committee; a block-only entry anywhere.
#[test]
fn every_node_serves_the_terminal_pair_of_every_passed_epoch_after_re_jumps() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    cfg.archive_scan = true;
    let end = 5 * EPOCH_LEN + 8;
    let out = Stand::new(cfg).run_until(move |p| p.min_height() >= end, Duration::from_secs(400));
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

    // PREMISE: the jumper jumped, its floor rose above earlier terminals, and
    // every node went at least two epochs past every terminal it is asked for.
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
    // Two epochs past `last(E)` means `start(E + 2)`; that holds for E ≤ 3 at
    // the stop height (epoch 5 + 8), and epoch 4's terminal is one epoch back.
    for i in 0..4 {
        assert!(
            out.heights[i] >= (epochs_passed + 1) * EPOCH_LEN,
            "node {i} at {} is not two epochs past last({}) = {}",
            out.heights[i],
            epochs_passed - 1,
            last(epochs_passed - 1)
        );
    }

    // OBSERVATION (а): every member of `committee(E+1)` — and here every node —
    // serves the pair at `last(E)` for every passed epoch.
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
    // OBSERVATION (б): every height a node holds carries its own certificate.
    for i in 0..4 {
        let (missing, block_only) = holes(&out, i);
        assert!(
            block_only.is_empty(),
            "node {i} holds blocks without their own finalization: {block_only:?}"
        );
        assert!(missing.is_empty(), "node {i} has holes: {missing:?}");
    }
}

/// (Д3, gate at 80 blocks) The same rotation, the re-jump gate widened so node 3
/// stays parked at 95 until the network is more than two epochs ahead. What the
/// run shows: its MARSHAL freezes at `last(epoch(95) + 2) = 159` — the ordering
/// plane's two-epoch ceiling (`.dpos-study` memory: `dpos ordering plane 2-epoch
/// ceiling`) — while the members run on; the jump then lands ABOVE the frozen tip
/// (`from: 95`, consumed tip ≥ 176) and leaves a HOLE in its archive: heights
/// above 159 up to a few below the landing are absent outright, and the ones the
/// marshal's backward parent walk pulled in from the consumed certificate are
/// block-only (no certificate of their own). The node recovers and finishes in
/// lockstep.
///
/// What this pins for the ladder: the pair a jumper can serve for a terminal it
/// jumped OVER exists only if `seed_boundary_below_floor` fetched it
/// (`executor.rs:2310-2365`), which needs `boundary_fetch` — `None` in this
/// stand — so the stand cannot witness the seeding; it CAN witness that nothing
/// else fills the hole. No terminal falls into the hole here (159 was archived
/// before the freeze, 191 after the landing), so every terminal pair is still
/// present on every node.
///
/// Falsifier: node 3 jumping before the network was two epochs ahead (consumed
/// tip < 176); its marshal tip not frozen at 159 (then the ceiling is not what
/// the memory says); no hole (then something back-filled the jumped range and
/// the ladder's seeding is not the only source); a terminal missing on any node;
/// node 3 not recovering.
#[test]
fn a_node_more_than_two_epochs_behind_freezes_its_marshal_at_the_ceiling_and_jumps_over_a_hole() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(80);
    cfg.archive_scan = true;
    let end = 8 * EPOCH_LEN + 8;
    let out = Stand::new(cfg).run_until(
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

    // PREMISE: one jump, from the park height, consuming a tip more than two
    // epochs above it, landing above the ceiling.
    let calls = &out.jump_calls[3];
    let landed: Vec<_> = calls.iter().filter(|c| c.outcome == "Landed").collect();
    assert_eq!(landed.len(), 1, "expected exactly one landing: {calls:?}");
    let jump = landed[0];
    assert_eq!(
        jump.from,
        last(2),
        "the jump did not start from the park height"
    );
    let (tip, _) = jump.consumed.expect("a landing consumed a certificate");
    let (landing, _) = jump.landed.expect("asserted Landed");
    let ceiling = last(2 + 2);
    assert!(
        tip > last(2) + 80,
        "the jump fired before the network was 80 blocks ahead: tip {tip}"
    );
    assert!(
        landing > ceiling,
        "the landing {landing} is not above the frozen tip {ceiling}"
    );
    let floor = out.metric(3, MARSHAL_FLOOR).expect("floor gauge");
    assert!(
        floor >= landing as f64,
        "floor {floor} below the landing {landing}"
    );

    // OBSERVATION: the ceiling — everything up to `last(4)` is archived with
    // its certificate, `last(4) + 1` is not — and the hole above it.
    for h in 1..=ceiling {
        let a = entry(&out, 3, h);
        assert!(
            a.finalization && a.block,
            "node 3 lacks the pair at {h} below the ceiling {ceiling}: {a:?}"
        );
    }
    assert!(
        missing.contains(&(ceiling + 1)),
        "the marshal did not freeze at the ceiling: {} is archived; missing={missing:?}",
        ceiling + 1
    );
    assert!(
        missing.iter().all(|h| *h > ceiling && *h < landing),
        "a hole outside (ceiling, landing): missing={missing:?}"
    );
    assert!(
        block_only.iter().all(|h| *h > ceiling && *h < tip),
        "a block-only entry outside (ceiling, consumed tip): {block_only:?}"
    );
    for h in tip..=out.heights[3] {
        let a = entry(&out, 3, h);
        assert!(
            a.finalization && a.block,
            "node 3 lacks the pair at {h} above the consumed tip {tip}: {a:?}"
        );
    }
    // Every terminal is still served by every node: none fell into the hole.
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

/// (Д4, measurement) `best − ordering_finalized` on an honest static stand, at
/// the default link and under a slow, lossy one. Prints the event-driven
/// histograms; asserts only that speculation happened at all (the FCU histogram
/// is non-empty on every node), which is the premise of the number.
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

/// (Д5, measurement) Distinct agreement bodies per `(epoch, sender)` when the
/// consensus plane is cut for six views in the middle of epoch 1's ceremony
/// (dealing ends at `32 + 12`, the agreement runs after it). Prints the count;
/// asserts the premise — the cut happened inside epoch 1 and the key for epoch
/// 2 was still minted — and nothing about the number.
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

/// (Д8) A peer tracked as SECONDARY on the real authenticated transport
/// (`commonware_p2p::authenticated::discovery`, run on the deterministic
/// runtime's in-memory sockets): three peers, every one tracking the same
/// `TrackedPeers { primary: {A, B}, secondary: {C} }` at index 0, B and C
/// bootstrapping to A. What is measured, each as its own fact: whether C's
/// inbound connection is accepted (A's `Recipients::One(C)` send reports C);
/// whether C receives a `Recipients::All` frame from A and from B; whether A
/// hears a frame C sends it. The doc block records the run's answer; the
/// asserts pin it.
///
/// Falsifier: any of the four facts flipping.
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
