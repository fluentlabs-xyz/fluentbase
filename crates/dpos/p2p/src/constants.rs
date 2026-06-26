//! Hardcoded protocol-wide constants — must be identical across the network.
//!
//! Any change requires a coordinated software release because all
//! validators must agree on these values byte-for-byte (the list:
//! `namespace`, `max_message_size`, `synchrony_bound`,
//! `max_peer_set_size`, `tracked_peer_sets`, `gossip_bit_vec_frequency`,
//! all timeouts, all rate-limit quotas). The constants below cover
//! every such item we control; `synchrony_bound`/`max_handshake_age`/etc
//! are left at commonware's `Config::recommended` defaults (verified
//! identical-across-network by virtue of the Config builder).

use commonware_runtime::Quota;
use commonware_utils::NZU32;

// Channel IDs
//
// Three top-level Muxed channels (per-epoch demux): VOTE/CERT/RESOLVER.
// Four top-level non-Muxed channels (global one-instance for the node):
// BROADCAST (block-data dissemination via `buffered::Engine`),
// MARSHAL (backfill via `marshal::resolver::p2p::init`), BEACON
// (randomness-beacon DKG; see BEACON_CHANNEL below), and BEACON_RESOLVER
// (DKG-log recovery via `commonware_resolver::p2p`; see BEACON_RESOLVER_CHANNEL
// below). Order is arbitrary but fixed: changing it without coordinated
// release silently misroutes consensus traffic across the network.
pub const VOTE_CHANNEL: u64 = 0;
pub const CERT_CHANNEL: u64 = 1;
pub const RESOLVER_CHANNEL: u64 = 2;
pub const BROADCAST_CHANNEL: u64 = 3;
pub const MARSHAL_CHANNEL: u64 = 4;
// Beacon plane (threshold randomness): carries the per-epoch self-DKG
// ceremony traffic (`BeaconMessage::Dkg`) that establishes `PK_epoch`. The
// recovered randomness SEED rides INSIDE the consensus cert
// (`CombinedCertificate`) — the old sign-at-notarize seed side-channel was
// deleted — so this channel carries DKG ONLY. A GLOBAL one-instance channel
// like BROADCAST/MARSHAL, registered once in `FluentP2P::build` and consumed by
// the live `DkgActor` (`dpos.rs::launch` → `beacon/actor.rs`). Per-epoch Muxing
// (so DKG-for-E and DKG-for-E+1 never interleave) is deferred.
pub const BEACON_CHANNEL: u64 = 5;
// Beacon-plane DKG-log recovery resolver (`commonware_resolver::p2p`): a
// mid-window-restarted committee member re-fetches the public dealer logs it
// never received, keyed by `{epoch, dealer}`, from peers that still hold them
// (the always-on plane keeps committee[E] connected; the EpochTransition's
// `registry ∪ committee` tracker keeps them in `latest.primary`). Replaces the
// former best-effort `BEACON_CHANNEL` LogRequest/LogResponse gossip pull. A
// GLOBAL one-instance channel like BROADCAST/MARSHAL, registered once in
// `FluentP2P::build` and consumed by the beacon-plane resolver engine
// (`node/dpos.rs::build_beacon_plane`). MUST be byte-identical across the
// network (a new channel id all nodes agree on).
pub const BEACON_RESOLVER_CHANNEL: u64 = 6;

// Per-channel rate quotas
//
// Aligned to alto/tempo precedent (tempo `config.rs:37-43`, alto
// `validator/main.rs:214-235`): 128/s per recipient pair for vote/cert/
// resolver. Previous derivation (10/s based on happy-path 3/s + 3× headroom)
// ignored view-change/nullify bursts and per-`Recipients::All` quota
// consumption at n=51 validators (each broadcast consumes 50 pair-slots).
// 128/s = 12.8× over Fluent's prior 10/s quota; alto/tempo use this same
// value as a widely-deployed default with no published load-test
// justification (cargo-cult from known-good precedent; measured trace
// deferred until production blocks exist).
//
// BROADCAST/MARSHAL: untouched (block-data infrequent + backfill bursty —
// alto/tempo also use 8/s for BROADCASTER_LIMIT).
pub const VOTE_QUOTA: Quota = Quota::per_second(NZU32!(128));
pub const CERT_QUOTA: Quota = Quota::per_second(NZU32!(128));
pub const RESOLVER_QUOTA: Quota = Quota::per_second(NZU32!(128));
// BROADCAST: block-data is fat but infrequent.
// MARSHAL:   backfill is request-bursty (catch-up).
pub const BROADCAST_QUOTA: Quota = Quota::per_second(NZU32!(8));
pub const MARSHAL_QUOTA: Quota = Quota::per_second(NZU32!(16));
// BEACON: DKG is bursty for one round per epoch (dealing/ack broadcast) then
// idle. Matched to VOTE/CERT (same n=51 fan-out for the DKG round).
pub const BEACON_QUOTA: Quota = Quota::per_second(NZU32!(128));
// BEACON_RESOLVER: DKG-log recovery fetch — request-bursty during a single
// restarted member's catch-up (≤ n keys, one per missing dealer), then idle.
// Matched to MARSHAL (the other resolver backfill channel).
pub const BEACON_RESOLVER_QUOTA: Quota = Quota::per_second(NZU32!(16));

// Per-channel backlog (mailbox size before back-pressure)
pub const VOTE_BACKLOG: usize = 256;
pub const CERT_BACKLOG: usize = 256;
pub const RESOLVER_BACKLOG: usize = 64;
pub const BROADCAST_BACKLOG: usize = 32;
pub const MARSHAL_BACKLOG: usize = 128;
pub const BEACON_BACKLOG: usize = 256;
pub const BEACON_RESOLVER_BACKLOG: usize = 128;

// Wire caps
//
// `MAX_MESSAGE_SIZE` covers absolute worst-case at current 50M gas
// (50_000_000 / 16 ≈ 3.125 MB calldata-heavy block) + ~30% headroom.
// Hardcoded (not chainspec-tunable) because all peers must agree.
pub const MAX_MESSAGE_SIZE: u32 = 4 * 1024 * 1024;

// Committee cap — bounds the COMMITTEE (extra_data `committee_size: u8`
// bitmap, BLS scheme building), NOT the p2p tracker feed (see
// `MAX_REGISTRY_PEER_SET` below for that).
//
// MUST mirror
// `solidity-contracts/contracts/staking/ChainConfig.sol::MAX_ACTIVE_VALIDATORS`
// and stay ≤ 255 (the u8 wire format). Drift between the two literals means a
// successful `ChainConfig.setActiveValidatorsLength` call later fails the
// startup cap assert (dpos.rs) or corrupts the attestation bitmap. Update
// both in the SAME PR.
pub const MAX_COMMITTEE_SIZE: u64 = 51;

// Tracker bit-vec guard for the tier-2 registry feed (the FULL Active
// validator registry ∪ current committee is tracked, not just the
// committee — every activated validator keeps consensus-plane
// connectivity). Generous, NOT policy: the registry is bounded
// economically (min stake) + by governance activation, and commonware's
// recommended `max_peer_set_size` is 2^16 (gossip costs one bit per
// peer). The staking-reader's `check_peer_set_size` rejects an oversize
// feed as a typed `ReadError::PeerSetTooLarge` instead of letting
// commonware's tracker panic deeper.
pub const MAX_REGISTRY_PEER_SET: u64 = 4096;

// Network policy
//
// `ALLOW_DNS: false` — Socket-only ingress; DNS provider out of trust
// path. Trust anchor = on-chain Ed25519 + handshake.
// Production rejects RFC-1918 ingress; this is network-derived in
// `FluentP2PConfig::into_commonware_config` (deployed networks → false).
pub const ALLOW_DNS: bool = false;

// Listen port
//
// Default 9000; runtime override via env var `FLUENT_DPOS_P2P_PORT`.
// Must NOT collide with reth devp2p :30303 or any reth RPC port.
pub const DEFAULT_LISTEN_PORT: u16 = 9000;
pub const LISTEN_PORT_ENV_VAR: &str = "FLUENT_DPOS_P2P_PORT";
