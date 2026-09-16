//! Hardcoded protocol-wide constants — every node on a network computes the same
//! values byte-for-byte, so any change requires a coordinated release.
//!
//! The commonware config fields this crate builds are under the same rule:
//! `namespace`, `max_message_size`, `synchrony_bound`, `max_peer_set_size`,
//! `tracked_peer_sets`, `gossip_bit_vec_frequency`, every timeout and every
//! rate-limit quota. The items not set here stay at commonware's
//! `Config::recommended` defaults, which every node computes identically.

use commonware_runtime::Quota;
use commonware_utils::NZU32;

// Channel ids.
pub const VOTE_CHANNEL: u64 = 0;
pub const CERT_CHANNEL: u64 = 1;
pub const RESOLVER_CHANNEL: u64 = 2;
pub const BROADCAST_CHANNEL: u64 = 3;
pub const MARSHAL_CHANNEL: u64 = 4;
// Beacon plane: the per-epoch self-DKG ceremony traffic (`BeaconMessage::Dkg`)
// that establishes `PK_epoch`. The recovered randomness seed rides inside the
// consensus cert (`CombinedCertificate`), so this channel carries DKG only.
pub const BEACON_CHANNEL: u64 = 5;
// DKG-log recovery resolver (`commonware_resolver::p2p`): a mid-window-restarted
// committee member re-fetches the public dealer logs it never received, keyed by
// `{epoch, dealer}`, from peers that still hold them.
pub const BEACON_RESOLVER_CHANNEL: u64 = 6;
// Plane-native `CertUpstream` frontier resolver (`commonware_resolver::p2p`): a
// validator with no WS upstream discovers the network frontier
// (`FrontierKey::Latest`) and pulls by-height finalizations
// (`FrontierKey::Finalized`) over the consensus plane.
pub const FRONTIER_CHANNEL: u64 = 7;
// Equivocation evidence gossip: a node republishes the signed votes it holds for
// a decided round, so the two halves of a split-delivered equivocation can meet.
pub const EVIDENCE_CHANNEL: u64 = 8;

// Sub-channel id space. The muxed channels carry sub-channels keyed by a `u64`;
// a per-epoch consensus engine registers the epoch number itself and the global
// singletons take 0, so the epoch-key agreement instance takes
// `DKG_SUBCHANNEL_BASE | target_epoch`, above every epoch a real chain reaches (at
// one epoch per day, 2^32 epochs is ~11.7 million years). The caller range-checks
// the epoch anyway, and a collision surfaces as the muxer's `AlreadyRegistered`
// rather than overwriting the live route.
pub const DKG_SUBCHANNEL_BASE: u64 = 1 << 32;

/// The epoch a sub-channel id denotes, or `None` when the id belongs to the
/// agreement slice. A muxer hands an unrouted frame's id back raw, so every
/// ingress that turns an id into an epoch must come through here.
pub const fn epoch_from_subchannel(id: u64) -> Option<u64> {
    if id >= DKG_SUBCHANNEL_BASE {
        None
    } else {
        Some(id)
    }
}

// Per-channel rate quotas, vote/cert/resolver aligned to the alto/tempo default of
// 128/s per recipient pair — a deployed precedent rather than a measured trace. A
// `Recipients::All` broadcast at n=51 consumes 50 pair-slots, so a lower quota
// would throttle view-change and nullify bursts.
pub const VOTE_QUOTA: Quota = Quota::per_second(NZU32!(128));
pub const CERT_QUOTA: Quota = Quota::per_second(NZU32!(128));
pub const RESOLVER_QUOTA: Quota = Quota::per_second(NZU32!(128));
// Block data is fat but infrequent; marshal backfill is request-bursty during
// catch-up.
pub const BROADCAST_QUOTA: Quota = Quota::per_second(NZU32!(8));
pub const MARSHAL_QUOTA: Quota = Quota::per_second(NZU32!(16));
// DKG is bursty for one round per epoch (dealing/ack broadcast), with the same
// n=51 fan-out as vote/cert.
pub const BEACON_QUOTA: Quota = Quota::per_second(NZU32!(128));
// DKG-log recovery is request-bursty during one restarted member's catch-up (≤ n
// keys, one per missing dealer), then idle; matched to marshal.
pub const BEACON_RESOLVER_QUOTA: Quota = Quota::per_second(NZU32!(16));
// A rare, cheap, per-peer tip/by-height query (one per jump attempt, re-jump edge
// or marshal backfill hole), answered from O(1) local reads; matched to the other
// resolver channels.
pub const FRONTIER_QUOTA: Quota = Quota::per_second(NZU32!(16));
// At most one publication per validator per failed view, bounded by the view rate
// rather than by traffic.
pub const EVIDENCE_QUOTA: Quota = Quota::per_second(NZU32!(16));

// Per-channel backlog: mailbox size before back-pressure.
pub const VOTE_BACKLOG: usize = 256;
pub const CERT_BACKLOG: usize = 256;
pub const RESOLVER_BACKLOG: usize = 64;
pub const BROADCAST_BACKLOG: usize = 32;
pub const MARSHAL_BACKLOG: usize = 128;
pub const BEACON_BACKLOG: usize = 256;
pub const BEACON_RESOLVER_BACKLOG: usize = 128;
pub const FRONTIER_BACKLOG: usize = 64;
pub const EVIDENCE_BACKLOG: usize = 64;

// Wire cap: the worst case at 50M gas (50_000_000 / 16 ≈ 3.125 MB of
// calldata-heavy block) plus ~30% headroom. Hardcoded rather than
// chainspec-tunable because all peers must agree.
pub const MAX_MESSAGE_SIZE: u32 = 4 * 1024 * 1024;

// Committee cap — bounds the committee (the production record's `leader_index: u8`,
// BLS scheme building), not the p2p tracker feed (see `MAX_REGISTRY_PEER_SET`
// below). The staking module enforces the same cap on the way in, so both sides
// import this one declaration.
pub use fluentbase_types::staking_protocol::MAX_COMMITTEE_SIZE;

// Tracker bit-vec guard for the tier-2 registry feed (the full Active validator
// registry ∪ current committee is tracked, not just the committee). Generous, not
// policy: the registry is bounded economically (min stake) and by governance
// activation, and commonware's recommended `max_peer_set_size` is 2^16. The
// staking-reader's `check_peer_set_size` rejects an oversize feed as a typed
// `ReadError::PeerSetTooLarge` instead of letting commonware's tracker panic
// deeper.
pub const MAX_REGISTRY_PEER_SET: u64 = 4096;

// Network policy: accept DNS-hostname ingress (`Ingress::Dns`) in both the locally
// advertised Info record and gossiped peer Info. DNS is not a trust anchor —
// identity is on-chain Ed25519 + handshake, and the resolved IP is re-checked
// against `allow_private_ips` after resolution.
//
// `Ingress::is_valid` rejects DNS-form Info when `allow_dns` is false, so a
// network split between the two values partitions on hostname records: all
// validators must upgrade together before any node advertises a hostname.
// IP-only Info stays interoperable either way.
pub const ALLOW_DNS: bool = true;

// Default listen port; runtime override via `LISTEN_PORT_ENV_VAR`. Must not
// collide with reth devp2p :30303 or any reth RPC port.
pub const DEFAULT_LISTEN_PORT: u16 = 9000;
pub const LISTEN_PORT_ENV_VAR: &str = "FLUENT_DPOS_P2P_PORT";
