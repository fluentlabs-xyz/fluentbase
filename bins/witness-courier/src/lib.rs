//! # witness-courier
//!
//! gRPC transport layer for the Fluent zkVM proving pipeline.
//!
//! This crate provides everything needed to move block witnesses from a
//! Fluent/Reth node to external proving infrastructure:
//!
//! - **[`proto`]** — generated protobuf/gRPC types from `witness.proto`.
//! - **[`types`]** — shared domain types ([`ProveRequest`](types::ProveRequest)).
//! - **[`hub`]** — [`WitnessHub`](hub::WitnessHub), an in-process ring buffer
//!   with broadcast fan-out. The ExEx pushes witnesses here; the gRPC server
//!   reads from here.
//! - **[`server`]** — gRPC server implementation. The node embeds this.
//! - **[`client`]** — gRPC client with automatic reconnect and checkpoint
//!   persistence. The sidecar binary uses this.
//!
//! ## Architecture
//!
//! ```text
//! ExEx ──push──▶ WitnessHub ◀──subscribe── gRPC server ──stream──▶ courier(s)
//!                (ring buf)
//! ```
//!
//! The hub holds up to 1024 recent witnesses. When a courier reconnects it
//! sends `Subscribe(from_block)` and receives a replay of buffered witnesses
//! followed by a live stream — zero gaps if downtime < buffer size.

/// Generated protobuf and gRPC types.
pub mod proto {
    tonic::include_proto!("fluent.witness.v1");
}

pub mod types;
pub mod hub;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "client")]
pub mod accumulator;

#[cfg(feature = "client")]
pub mod l1_listener;

#[cfg(feature = "client")]
pub mod l1_submitter;