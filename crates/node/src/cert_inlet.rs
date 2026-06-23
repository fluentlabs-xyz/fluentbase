//! Node-side cert-inlet wiring (the validator-side second producer).
//!
//! The cert-inlet ([`fluentbase_consensus::CertInlet`]) is the SECOND producer
//! into a node's singleton marshal: it BLS-verifies an upstream
//! `(Finalization, OrderBlock)` against the on-chain committee and `report()`s
//! it, driving the executor (the sole reth writer) exactly as a locally-formed
//! finalization would. The inlet itself writes NOTHING to reth.
//!
//! An upstream-configured `--dpos` validator runs this against its own marshal
//! (the local BFT engine being the first producer). While the node is a
//! committee member its locally-formed certs lead and the inlet's reports are
//! duplicates the marshal absorbs below its floor; once it rotates out,
//! `reconcile_roles` keeps it a Verifier and the inlet drives the base it
//! follows — the production-path fix for a rotated-out validator.
//! (A NON-validator follower uses the consensus crate's `launch_follower`
//! near-planeless path, which spawns its own inlet.)

use alloy_consensus::Header;
use alloy_primitives::B256;
use commonware_runtime::{tokio::Context, Handle, Metrics as _, Spawner as _};
use fluentbase_consensus::{
    cert_inlet::LiveFrontierTee, CertInlet, CertUpstream as _, CommitteeSource, MarshalMailbox,
    RethCommitteeSource, RotateUpstream,
};
use fluentbase_staking_reader::reader::{RethStakingStateReader, StakingReaderConfig};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::ConfigureEvm;
use reth_storage_api::{HeaderProvider, StateProviderFactory};
use std::sync::Arc;
use tracing::{error, info};

/// Build the validator-path inlet's [`CommitteeSource`] over the node's own reth
/// state. Collapses onto the consensus crate's `pub`
/// [`fluentbase_consensus::RethCommitteeSource`] (the follower path already uses
/// it) — the node only owns the reth-typed `RethStakingStateReader` construction.
pub(crate) fn committee_source<Provider, EvmConfig>(
    provider: Provider,
    evm_config: EvmConfig,
    staking_config: StakingReaderConfig,
    chain_id: u64,
    finalized_hash: Arc<dyn Fn() -> Option<B256> + Send + Sync>,
) -> RethCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    RethCommitteeSource::new(
        RethStakingStateReader::new(provider, evm_config, staking_config),
        chain_id,
        finalized_hash,
    )
}

/// Spawn the cert-inlet shadow task: subscribe to the upstream WS, feed every
/// live `(Finalization, OrderBlock)` through [`CertInlet::ingest`] into the same
/// marshal the local engine drives.
///
/// `tee` re-homes the live-frontier cursors (`live_height` + the DkgActor deal
/// clock) off the verified upstream tip — present on a validator (it owns the
/// beacon plane), so committee[E+1] resolves and the DKG deals at the LIVE
/// frontier rather than this node's lagging EL-finalized state.
///
/// Fail-closed-on-TOTAL-loss (Risk-3): a single bad cert is skipped inside
/// `ingest` (WARN + `Ok`), but if the WS `finalized_rx` closes (every upstream
/// URL dead) the loop breaks → the returned `Handle` resolves → the supervisor
/// `select!` arm fires fatal (cancels the shutdown token). A live-but-bad stream
/// stalls the marshal naturally; only total stream loss is the loud exit.
pub(crate) fn spawn_cert_inlet<C>(
    ctx: Context,
    marshal: MarshalMailbox,
    committees: C,
    urls: Vec<String>,
    tee: LiveFrontierTee,
) -> Handle<()>
where
    C: CommitteeSource,
{
    ctx.with_label("cert_inlet").spawn(move |c| async move {
        let (ws_actor, upstream_handle, mut finalized_rx, conn_gen) =
            crate::cert_follow::upstream::init(c.clone(), urls);
        let mut ws_handle = ws_actor.start();
        // DATA-fault rotation trigger (#7): after MAX_UPSTREAM_FAULTS consecutive
        // unverifiable certs over a healthy connection, rotate to the next
        // configured upstream URL. `rotate()` drops the connection so the WS
        // actor's run loop advances. Connection-level failover can never see a
        // bad PAYLOAD on a live connection — this is the only signal for it.
        let rotate: RotateUpstream = upstream_handle.rotate_callback();
        // Keep the request handle alive for the inlet's whole lifetime (the WS
        // actor's run loop exits the instant ALL handles drop); the rotate
        // closure holds one clone, this binding holds the other.
        let _upstream_keepalive = upstream_handle;
        // The inlet ALWAYS BLS-verifies (no no-verify mode in v1). The
        // connection-generation token scopes the data-fault streak to the LIVE
        // connection (#7) so a connection-level auto-rotation does not carry one
        // upstream's faults into the next URL's rotation budget.
        let mut inlet = CertInlet::new(marshal, committees, c)
            .with_tee(tee)
            .with_rotate(rotate)
            .with_connection_token(conn_gen);
        info!("cert-inlet SHADOW producer started");
        loop {
            tokio::select! {
                uf = finalized_rx.recv() => match uf {
                    Some(uf) => {
                        if let Err(e) = inlet.ingest(uf).await {
                            error!(error = ?e, "cert-inlet fatal (committee read)");
                            break;
                        }
                    }
                    None => {
                        error!("cert-inlet WS stream closed (all upstreams dead); exiting fatal");
                        break;
                    }
                },
                r = &mut ws_handle => {
                    error!(result = ?r, "cert-inlet WS actor exited; exiting fatal");
                    break;
                }
            }
        }
    })
}
