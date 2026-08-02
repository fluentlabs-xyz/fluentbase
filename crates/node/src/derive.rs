//! [`RethBlockDeriver`] — the node-side [`DerivedBlockBuilder`]: executes an
//! agreed `OrderBlock`'s txs on the parent's post-state via reth-evm's
//! `BlockBuilder` (the stock payload builder's execution path) and seals the
//! derived Ethereum block. Determinism contract: every input is agreed data
//! (the OrderBlock fields) or derived state — never local config.

use alloy_consensus::Header;
use alloy_primitives::B256;
use eyre::WrapErr as _;
use fluentbase_consensus::beacon::seed::{prev_randao_from_seed, Seed};
use fluentbase_consensus::{
    DerivedBlock, DerivedBlockBuilder, FaultClass, OrderBlock, ParentHeaderMissing,
};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{
    execute::{BlockBuilder as _, BlockExecutionError, BlockExecutionOutput, BlockValidationError},
    ConfigureEvm, NextBlockEnvAttributes,
};
use reth_primitives_traits::{RecoveredBlock, SealedHeader, SignedTransaction as _};
use reth_revm::{database::StateProviderDatabase, db::State};
use reth_storage_api::{HeaderProvider, StateProviderFactory};
use reth_trie_common::{updates::TrieUpdates, HashedPostState};

/// One derivation's full output: the recovered block plus every execution
/// artifact the engine-tree needs to import it WITHOUT re-executing
/// (`EngineApiRequest::InsertExecutedBlock`). The consensus crate sees it
/// only through [`DerivedBlock`] (hash + number).
#[derive(Debug)]
pub struct DerivedExecution {
    pub recovered: RecoveredBlock<reth_ethereum_primitives::Block>,
    pub output: BlockExecutionOutput<reth_ethereum_primitives::Receipt>,
    pub hashed_state: HashedPostState,
    pub trie_updates: TrieUpdates,
    /// Beacon observation for the executor's `BeaconMetrics`: `Some(true)` =
    /// threshold seed present (`prev_randao = H(seed)`), `None` = no seed
    /// (pre-beacon / fallback). See [`resolve_prev_randao`].
    pub beacon_active: Option<bool>,
}

impl DerivedBlock for DerivedExecution {
    fn evm_hash(&self) -> B256 {
        self.recovered.hash()
    }

    fn number(&self) -> u64 {
        self.recovered.number
    }

    fn beacon_active(&self) -> Option<bool> {
        self.beacon_active
    }
}

#[derive(Clone, Debug)]
pub struct RethBlockDeriver<Client, Evm> {
    client: Client,
    evm_config: Evm,
}

impl<Client, Evm> RethBlockDeriver<Client, Evm> {
    pub fn new(client: Client, evm_config: Evm) -> Self {
        Self { client, evm_config }
    }
}

/// Decide `prev_randao` from the cert-recovered seed: `H(seed) = keccak256(σ)`
/// when a seed is present, else the weak deterministic `fallback`. The seed is a
/// unique threshold signature already bound by the multisig finalization cert
/// (recovered from ≥t partials each checked at vote time), so the deriver trusts
/// it — the on-chain `PK_E` re-verification carried no security and is gone
/// (DPOS_ARCHITECTURE §8.11). No key, no read, no `Result`. Free fn so it runs
/// inside `spawn_blocking`.
///
/// Returns `(prev_randao, beacon_active)`: `Some(true)` = threshold randomness,
/// `None` = no seed (pre-beacon / fallback). Pure keccak (cheap) — resolved on
/// the async path (not inside `spawn_blocking`) so the per-block derive-seed
/// telemetry can log the resolved `prev_randao` next to the resulting hash.
fn resolve_prev_randao(seed: Option<&Seed>, fallback: B256) -> (B256, Option<bool>) {
    match seed {
        Some(s) => {
            let prev_randao = prev_randao_from_seed(s);
            tracing::info!(round = ?s.target_round, %prev_randao, "beacon: threshold prev_randao active");
            (prev_randao, Some(true))
        }
        None => (fallback, None),
    }
}

impl<Client, Evm> DerivedBlockBuilder for RethBlockDeriver<Client, Evm>
where
    Client: StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>
        + Clone
        + 'static,
{
    type Derived = DerivedExecution;

    async fn derive_and_execute(
        &self,
        order: OrderBlock,
        parent_evm_hash: B256,
        seed: Option<Seed>,
    ) -> eyre::Result<DerivedExecution> {
        let client = self.client.clone();
        let evm_config = self.evm_config.clone();
        let fallback = order.digest().0;
        // Derive-seed telemetry: the executed height + the seed ROUND this derive
        // consumes, captured before `order`/`seed` move into the blocking task.
        let height = order.height;
        let seed_round = seed.as_ref().map(|s| s.target_round);
        // Resolve `prev_randao` HERE (pure keccak, non-behavioral) so the telemetry
        // line below can tie the resolved randomness to the resulting block hash.
        let (prev_randao, beacon_active) = resolve_prev_randao(seed.as_ref(), fallback);
        // EVM execution + state-root are CPU-bound (~V per block) — keep off the
        // async worker threads. The transient belt re-runs the WHOLE derive on
        // any torn static-file manifestation (changeset-row Decode /
        // InconsistentData range / changeset-offsets short-read UnexpectedEof /
        // InconsistentSnapshot refusal) or the convergent mdbx read-tx timeout,
        // each under its own budget (see `with_transient_derive_retry`); a
        // clean run costs nothing extra.
        let derived_res = tokio::task::spawn_blocking(move || {
            with_transient_derive_retry(height, std::thread::sleep, || {
                derive_sync(
                    &client,
                    &evm_config,
                    &order,
                    parent_evm_hash,
                    prev_randao,
                    beacon_active,
                )
            })
        })
        .await;
        let derived = derived_res.wrap_err("derive task panicked")??;
        // DERIVE-SEED TELEMETRY (fork-root byte-confirm): ONE line per derived
        // block — the single chokepoint for ALL derive paths (speculative,
        // finalized re-derive, gap-walk, cert-follow) — tying executed height →
        // seed round consumed → resolved prev_randao → resulting EVM block hash.
        // Comparing a diverged node's line for a height against a good node's shows
        // whether the SAME seed round yielded different hashes (seed byte-mismatch)
        // or the two consumed different seed rounds.
        tracing::info!(
            target: "dpos::derive_seed",
            height,
            seed_round = ?seed_round,
            %prev_randao,
            evm_hash = %derived.evm_hash(),
            beacon_active = ?beacon_active,
            "derive-seed: block derived",
        );
        Ok(derived)
    }
}

/// FAST-phase bound/backoff for the transient static-file torn-read belts on
/// the derive path (the parent-header read and the whole-derive changeset
/// decode retry): reth's persistence thread freezes/appends static-file
/// segments concurrently with the executor's derive, and a read that lands
/// mid-freeze can observe torn mmap state. The common append/remap window is
/// sub-125 ms, so this short bounded phase absorbs it; the whole-derive belt
/// escalates to the SLOW phase below before exhausting (the parent-header belt
/// keeps this single phase — a header read is cheap and its window is small).
/// After exhaustion the error propagates EXACTLY as an unretried failure
/// (genuine on-disk corruption surfaces the same way and must stay loud — the
/// caller's fatal path is unchanged).
const TRANSIENT_READ_ATTEMPTS: u32 = 5;
const TRANSIENT_READ_BACKOFF: std::time::Duration = std::time::Duration::from_millis(25);

/// SLOW-phase bound/backoff for the whole-derive torn-read belt, entered after
/// the fast phase ([`TRANSIENT_READ_ATTEMPTS`]) exhausts. A single
/// mmap-append/remap window on a MULTI-HUNDRED-MB static-file segment under IO
/// load can exceed the fast phase's 125 ms budget (bundle-20260717T074614Z: a
/// ~152 MB segment torn read — `attempted to read an inconsistent data range
/// 152247225..0, data size: 152247702` — killed validator-2 despite matching
/// the classifier, i.e. the fast belt exhausted mid-append rather than the
/// classifier missing). `TORN_SLOW_ATTEMPTS × TORN_SLOW_BACKOFF` ≈ 3 s bounds
/// any single append; genuine corruption still exhausts and propagates loud.
const TORN_SLOW_ATTEMPTS: u32 = 10;
const TORN_SLOW_BACKOFF: std::time::Duration = std::time::Duration::from_millis(300);

/// Total torn-read attempts across both phases (fast + slow): the whole-derive
/// belt exhausts here. `5 × 25 ms + 10 × 300 ms ≈ 3.1 s`.
const TORN_TOTAL_ATTEMPTS: u32 = TRANSIENT_READ_ATTEMPTS + TORN_SLOW_ATTEMPTS;

/// The four stable torn-static-file-read display shapes. SINGLE source of truth in
/// `fluentbase-staking-reader` (the lower crate; its read-boundary classifier maps the
/// same shapes to `ReadError::TransientStorage`), re-imported here so the whole-derive
/// retry belt below and the staking read-boundary agree byte-for-byte:
/// - `DECODE_ERROR_DISPLAY`: reth's `DatabaseError::Decode` (torn changeset row — the
///   former `StorageBeforeTx::from_compact` panic);
/// - `TORN_RANGE_DISPLAY`: `NippyJarError::InconsistentData` (inverted/out-of-bounds
///   range read of a concurrently-appended segment, `ProviderError::Other(AnyError)`);
/// - `SHORT_READ_DISPLAY`: a `std::io::Error` `UnexpectedEof` short `read_exact_at` of
///   a still-appending static-file sidecar;
/// - `SNAPSHOT_DISPLAY`: `NippyJarError::InconsistentSnapshot` (committed-snapshot
///   validation refusal — active-writer self-load / `LoadedJar` reload-once failure).
use fluentbase_staking_reader::error::{
    DECODE_ERROR_DISPLAY, SHORT_READ_DISPLAY, SNAPSHOT_DISPLAY, TORN_RANGE_DISPLAY,
};

/// Classifies a derive failure as a torn static-file read — FOUR reth-fork-guard
/// manifestations, all raised while the persistence thread appends/freezes a
/// segment concurrently with `derive_sync` (execution state reads / `finish`'s
/// state-root + trie-updates compute, incl. the trie changeset-cache MISS
/// fallback `compute_block_trie_changesets` → `storage_changesets_range`):
///
/// 1. a torn changeset ROW (former `from_compact` panic) →
///    `ProviderError::Database(DatabaseError::Decode)`;
/// 2. a torn offset RANGE (former `Ok(None)`-swallow at the cursor) →
///    `ProviderError::Other(AnyError)` over `NippyJarError::InconsistentData`;
/// 3. a SHORT READ of a static-file sidecar (the changeset-offsets `read_exact_at`
///    reading a still-appending offsets file) → a `std::io::Error` of kind
///    `UnexpectedEof` (display "failed to fill whole buffer"), which surfaces at
///    `finish` (`derive.rs`'s `builder.finish` call) buried under eyre context;
/// 4. a committed-snapshot validation refusal (active-writer self-load /
///    `LoadedJar` reload-once failure) → `ProviderError::Other(AnyError)` over
///    `NippyJarError::InconsistentSnapshot` (see [`SNAPSHOT_DISPLAY`]).
///
/// Typed downcast over the source chain first (`ProviderError` / `DatabaseError`
/// / `std::io::Error`; for (2) the variant is matched typed but its payload by
/// display — `AnyError::source()` skips the wrapped error as a chain link and a
/// full `NippyJarError` downcast would need a new direct reth-git dep; for (3)
/// the `io::Error` is usually preserved as a chain link, matched on
/// `ErrorKind::UnexpectedEof`, but an `AnyError` wrapper is also matched by
/// display); display-substring fallback second, because two boundary wrappers
/// erase the typed chain (`InternalBlockExecutionError::EVM` embeds the source as
/// `{error}` text; `eyre!("...: {e}")` stringifies). The generic-io
/// `"failed to fill whole buffer"` display is safe here BECAUSE this classifier
/// is reachable ONLY through the belt that wraps `derive_sync` (nothing else
/// calls it): any `UnexpectedEof` observed mid-derive from a reth static-file
/// read IS a torn/transient shape (the append settles sub-second). Genuine
/// corruption raises the SAME errors and is indistinguishable — the caller's
/// retry MUST stay bounded (it exhausts the budget and propagates unchanged).
fn is_transient_torn_static_file_read(err: &eyre::Report) -> bool {
    use reth_storage_api::errors::{db::DatabaseError, ProviderError};
    err.chain().any(|e| {
        if let Some(p) = e.downcast_ref::<ProviderError>() {
            return match p {
                ProviderError::Database(DatabaseError::Decode) => true,
                ProviderError::Other(inner) => {
                    let display = inner.to_string();
                    display.contains(TORN_RANGE_DISPLAY)
                        || display.contains(SHORT_READ_DISPLAY)
                        || display.contains(SNAPSHOT_DISPLAY)
                }
                _ => false,
            };
        }
        if let Some(d) = e.downcast_ref::<DatabaseError>() {
            return matches!(d, DatabaseError::Decode);
        }
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            return io.kind() == std::io::ErrorKind::UnexpectedEof;
        }
        let display = e.to_string();
        display.contains(DECODE_ERROR_DISPLAY)
            || display.contains(TORN_RANGE_DISPLAY)
            || display.contains(SHORT_READ_DISPLAY)
            || display.contains(SNAPSHOT_DISPLAY)
    })
}

/// reth's custom non-MDBX code for `reth_libmdbx::Error::ReadTransactionTimeout`
/// (`libmdbx-rs/src/error.rs`: `Self::ReadTransactionTimeout => -96000`) — the
/// ONLY error on that code (`-96001` is the distinct `BotchedTransaction`).
const MDBX_READ_TX_TIMEOUT_CODE: i32 = -96000;

/// Stable display of `reth_libmdbx::Error::ReadTransactionTimeout` — carried
/// verbatim inside `DatabaseErrorInfo.message`. Deliberately keeps the "read"
/// prefix so other transaction failures never match.
const MDBX_READ_TX_TIMEOUT_DISPLAY: &str = "read transaction has been timed out";

/// True iff this `DatabaseError` carries the mdbx read-transaction timeout.
/// The libmdbx error is folded into `DatabaseErrorInfo { message, code }` at
/// each db-api call site under a per-site VARIANT — `Open` on the tx
/// `get`/dbi paths, `Read` on the cursor paths (the changeset-range walk) —
/// so the match keys on the CODE across every info-carrying variant, not on
/// the variant.
fn database_error_is_read_timeout(err: &reth_storage_api::errors::db::DatabaseError) -> bool {
    use reth_storage_api::errors::db::DatabaseError as E;
    match err {
        E::Open(i)
        | E::CreateTable(i)
        | E::Read(i)
        | E::Delete(i)
        | E::Commit(i)
        | E::InitTx(i)
        | E::InitCursor(i)
        | E::Stats(i) => i.code == MDBX_READ_TX_TIMEOUT_CODE,
        _ => false,
    }
}

/// Classifies a derive failure as the CONVERGENT mdbx read-transaction
/// timeout: on a cold-resumed node with a deep derive backlog, the trie
/// changeset-cache walk inside ONE `derive_and_execute` can exceed reth's
/// 300 s MDBX max-read-transaction limit — the reader-monitor resets the txn
/// and the operation returns `reth_libmdbx::Error::ReadTransactionTimeout`.
/// Unlike the torn reads (instant, sub-second window) this is REAL WORK that
/// timed out: retrying with a fresh txn resumes strictly WARMER, because the
/// changeset cache inserts PER BLOCK during the walk (reth `changesets.rs`:
/// compute → insert before returning; unbounded, engine-tree-lifetime,
/// evicted only on canonical persistence) — so a retry converges and cannot
/// loop at the same height. Same two-tier match as the torn-read classifier:
/// typed (`DatabaseErrorInfo.code == -96000` under any info-carrying variant)
/// first, display-substring fallback for stringifying wrappers.
fn is_convergent_db_timeout(err: &eyre::Report) -> bool {
    use reth_storage_api::errors::{db::DatabaseError, ProviderError};
    err.chain().any(|e| {
        if let Some(p) = e.downcast_ref::<ProviderError>() {
            return matches!(p, ProviderError::Database(d) if database_error_is_read_timeout(d));
        }
        if let Some(d) = e.downcast_ref::<DatabaseError>() {
            return database_error_is_read_timeout(d);
        }
        e.to_string().contains(MDBX_READ_TX_TIMEOUT_DISPLAY)
    })
}

/// Retry budget for the CONVERGENT mdbx read-tx timeout class — deliberately
/// MUCH larger than [`TRANSIENT_READ_ATTEMPTS`] and with NO sleep between
/// attempts: each attempt is up to 300 s of real changeset-walk work (backoff
/// is pointless), per-attempt forward progress is GUARANTEED by the per-block
/// changeset-cache insert (see [`is_convergent_db_timeout`]), and 64 × 300 s
/// bounds the worst cold-catch-up at ~5.3 h before still failing loud.
const CONVERGENT_DB_TIMEOUT_ATTEMPTS: u32 = 64;

/// Bounded whole-derive retry on the two transient failure classes above,
/// each with its OWN budget (per-class failure counters in one loop; a mixed
/// sequence never cross-contaminates the budgets):
///
/// - torn static-file reads (`Decode` row / `InconsistentData` range /
///   `UnexpectedEof` changeset-offsets short read / `InconsistentSnapshot`
///   refusal): a TWO-PHASE budget —
///   [`TRANSIENT_READ_ATTEMPTS`] × [`TRANSIENT_READ_BACKOFF`] (fast, the common
///   sub-125 ms append window) then [`TORN_SLOW_ATTEMPTS`] ×
///   [`TORN_SLOW_BACKOFF`] (slow, a multi-hundred-MB segment append under IO
///   load), exhausting at [`TORN_TOTAL_ATTEMPTS`];
/// - convergent mdbx read-tx timeouts: [`CONVERGENT_DB_TIMEOUT_ATTEMPTS`],
///   no sleep (each attempt is minutes of convergent work).
///
/// `sleep` is injected (production passes `std::thread::sleep`; tests pass a
/// no-op or recording seam) so the two-phase timing is unit-testable without
/// real backoff. Blocking sleeps — runs inside `spawn_blocking`. The derive is re-run whole
/// because the failures surface mid-execution or inside `finish` — there is
/// no resumable read site. Any non-matching error propagates immediately;
/// exhaustion propagates the LAST error unchanged. Metrics, only when a retry
/// actually happened:
/// `dpos_derive_changeset_decode_retry_total{outcome=recovered|exhausted}`
/// for the torn class (name kept from the first manifestation for series
/// continuity; the per-retry `debug!` carries the error display) and
/// `dpos_derive_db_timeout_retry_total{outcome=recovered|exhausted}` for the
/// timeout class, plus per-attempt `dpos_derive_db_timeout_attempts_total`
/// (ticked at the start of each timeout-class retry, with one `info!` line —
/// external liveness signal during a multi-minute convergence); one `info!`
/// per recovery with height + per-class attempts.
/// Map a derive failure into the family-5 fault taxonomy at the boundary that
/// OWNS reth's concrete error types: a torn static-file read is
/// [`FaultClass::TransientBounded`] (bounded — the trigger ALSO fires on genuine
/// corruption, so the belt exhausts loud), a mdbx read-txn timeout is
/// [`FaultClass::TransientConvergent`] (guaranteed per-attempt forward
/// progress). `None` = not a retry class → propagate immediately. The two
/// classifiers stay the leaf mappers; the display-substring fallbacks live HERE,
/// next to the reth types they parse — never in consensus code (invariant 3).
fn classify_derive_fault(err: &eyre::Report) -> Option<FaultClass> {
    if is_transient_torn_static_file_read(err) {
        Some(FaultClass::TransientBounded)
    } else if is_convergent_db_timeout(err) {
        Some(FaultClass::TransientConvergent)
    } else {
        None
    }
}

fn with_transient_derive_retry<T>(
    height: u64,
    mut sleep: impl FnMut(std::time::Duration),
    mut derive: impl FnMut() -> eyre::Result<T>,
) -> eyre::Result<T> {
    let mut torn_failures = 0u32;
    let mut torn_waited = std::time::Duration::ZERO;
    let mut timeout_failures = 0u32;
    loop {
        match derive() {
            Ok(v) => {
                if torn_failures > 0 {
                    metrics::counter!(
                        "dpos_derive_changeset_decode_retry_total",
                        "outcome" => "recovered"
                    )
                    .increment(1);
                }
                if timeout_failures > 0 {
                    metrics::counter!(
                        "dpos_derive_db_timeout_retry_total",
                        "outcome" => "recovered"
                    )
                    .increment(1);
                }
                if torn_failures > 0 || timeout_failures > 0 {
                    tracing::info!(
                        height,
                        torn_attempts = torn_failures,
                        timeout_attempts = timeout_failures,
                        "derive: recovered from transient derive failure(s)"
                    );
                }
                return Ok(v);
            }
            Err(e) if classify_derive_fault(&e) == Some(FaultClass::TransientBounded) => {
                torn_failures += 1;
                if torn_failures >= TORN_TOTAL_ATTEMPTS {
                    metrics::counter!(
                        "dpos_derive_changeset_decode_retry_total",
                        "outcome" => "exhausted"
                    )
                    .increment(1);
                    tracing::warn!(
                        height,
                        attempts = torn_failures,
                        waited_ms = torn_waited.as_millis() as u64,
                        error = format!("{e:#}"),
                        "derive: torn-read-class error persists after the two-phase retry \
                         belt (fast 5×25ms + slow 10×300ms) — propagating"
                    );
                    return Err(e);
                }
                // Two-phase backoff: the fast phase (≤ TRANSIENT_READ_ATTEMPTS)
                // absorbs the common sub-125 ms append/remap window; the slow
                // phase gives a single mmap-append/remap of a multi-hundred-MB
                // static-file segment under IO load room to settle (a ~152 MB
                // segment append under SATA + blaster load can exceed 125 ms —
                // bundle-20260717T074614Z). ~3.1 s total bounds any single
                // append; genuine corruption still exhausts and propagates.
                let backoff = if torn_failures <= TRANSIENT_READ_ATTEMPTS {
                    TRANSIENT_READ_BACKOFF
                } else {
                    TORN_SLOW_BACKOFF
                };
                torn_waited += backoff;
                tracing::debug!(
                    height,
                    attempt = torn_failures,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %e,
                    "derive: transient torn static-file read, retrying"
                );
                sleep(backoff);
            }
            Err(e) if classify_derive_fault(&e) == Some(FaultClass::TransientConvergent) => {
                timeout_failures += 1;
                if timeout_failures >= CONVERGENT_DB_TIMEOUT_ATTEMPTS {
                    metrics::counter!(
                        "dpos_derive_db_timeout_retry_total",
                        "outcome" => "exhausted"
                    )
                    .increment(1);
                    tracing::warn!(
                        height,
                        attempts = timeout_failures,
                        error = format!("{e:#}"),
                        "derive: mdbx read-tx timeout persists after the convergent \
                         retry budget — propagating"
                    );
                    return Err(e);
                }
                // No sleep: the next attempt resumes against the warmed
                // changeset cache immediately. Per-attempt visibility (soak
                // 2026-07-16: a converging retry looked like a 14-min stall at
                // INFO level): tick the attempts counter at the START of each
                // retry attempt — the soak harness scrapes it as
                // `reth_dpos_derive_db_timeout_attempts_total` — and log at
                // info! (inherently rate-limited: attempts are ≥ 300 s apart).
                metrics::counter!("dpos_derive_db_timeout_attempts_total").increment(1);
                tracing::info!(
                    height,
                    attempt = timeout_failures,
                    budget = CONVERGENT_DB_TIMEOUT_ATTEMPTS,
                    error = %e,
                    "derive: mdbx read-tx timeout (convergent — changeset cache \
                     warmed per block), retrying with a fresh read txn"
                );
            }
            other => return other,
        }
    }
}

/// Transient static-file-miss retry belt for the parent-header read: a header
/// read that lands mid-freeze can observe a torn offset table. The reth fork
/// guards that read (panic → `NippyJarError::InconsistentData`). Pinned-reth
/// vs fixed-reth split: against the CURRENT pin, `StaticFileCursor::get`
/// swallows the cursor error into `Ok(None)` — the torn read surfaces as a
/// spurious "header not found" and THIS belt absorbs it; once the reth-fork
/// cursor fix lands, the same torn read surfaces as
/// `Err(ProviderError::Other(InconsistentData))`, passes through here
/// UNRETRIED (a provider `Err` is not this belt's class), and is absorbed by
/// the OUTER [`with_transient_derive_retry`] belt instead (its classifier matches the
/// display) — this belt's live purpose then narrows to genuine `Ok(None)` from
/// the eager-canon visibility lag. Retries [`TRANSIENT_READ_ATTEMPTS`] ×
/// [`TRANSIENT_READ_BACKOFF`] before propagating the exact same
/// [`ParentHeaderMissing`] as an unretried miss (genuine absence / corruption
/// stays loud — the caller's fatal path is unchanged).
///
/// Runs `read` up to [`TRANSIENT_READ_ATTEMPTS`] times, sleeping `backoff` between
/// `Ok(None)` results (blocking sleep — `derive_sync` runs inside
/// `spawn_blocking`). Counts `dpos_derive_parent_read_retry_total`
/// (`outcome="recovered"|"exhausted"`) only when a retry actually happened.
fn parent_header_with_transient_retry<F, E>(
    mut read: F,
    parent_evm_hash: B256,
    backoff: std::time::Duration,
) -> eyre::Result<Header>
where
    F: FnMut() -> Result<Option<Header>, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    for attempt in 1..=TRANSIENT_READ_ATTEMPTS {
        if let Some(header) = read().wrap_err("read parent header")? {
            if attempt > 1 {
                metrics::counter!("dpos_derive_parent_read_retry_total", "outcome" => "recovered")
                    .increment(1);
            }
            return Ok(header);
        }
        if attempt < TRANSIENT_READ_ATTEMPTS {
            tracing::debug!(
                parent = %parent_evm_hash,
                attempt,
                "derive: parent header not found (transient static-file miss?), retrying"
            );
            std::thread::sleep(backoff);
        }
    }
    metrics::counter!("dpos_derive_parent_read_retry_total", "outcome" => "exhausted").increment(1);
    tracing::warn!(
        parent = %parent_evm_hash,
        attempts = TRANSIENT_READ_ATTEMPTS,
        "derive: parent header still missing after retry belt — propagating as missing"
    );
    Err(ParentHeaderMissing(parent_evm_hash).into())
}

fn derive_sync<Client, Evm>(
    client: &Client,
    evm_config: &Evm,
    order: &OrderBlock,
    parent_evm_hash: B256,
    prev_randao: B256,
    beacon_active: Option<bool>,
) -> eyre::Result<DerivedExecution>
where
    Client: StateProviderFactory + HeaderProvider<Header = Header>,
    Evm: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
{
    let parent_header = parent_header_with_transient_retry(
        || client.header(parent_evm_hash),
        parent_evm_hash,
        TRANSIENT_READ_BACKOFF,
    )?;
    let parent_sealed = SealedHeader::new(parent_header, parent_evm_hash);

    // The header read above just proved the parent exists, so a state-read
    // failure for the same hash is (overwhelmingly) the same eager-canonicalization
    // visibility lag — type it so the recovery/jump retry can absorb it; the
    // underlying provider error stays in the chain for the timeout report.
    let state_provider = client
        .state_by_block_hash(parent_evm_hash)
        .map_err(|e| eyre::Report::new(e).wrap_err(ParentHeaderMissing(parent_evm_hash)))?;
    let mut db = State::builder()
        .with_database(StateProviderDatabase::new(state_provider.as_ref()))
        .with_bundle_update()
        .build();

    // Field mapping mirrors the live chain's attrs builder
    // (`FluentPayloadAttributesBuilder::build_attrs`) except the
    // node-local values it used: prev_randao (was `B256::random()`) is the
    // beacon-resolved value (`H(seed(h))` or the gated `order.digest()`
    // fallback, decided by the caller), and timestamp/fee_recipient/gas_limit
    // come from the agreed OrderBlock.
    let attrs = NextBlockEnvAttributes {
        timestamp: order.timestamp,
        suggested_fee_recipient: order.fee_recipient,
        prev_randao,
        gas_limit: order.gas_limit,
        parent_beacon_block_root: Some(B256::ZERO),
        withdrawals: None,
        // The agreed `extra_data` — the 2-byte production record the proposer
        // stamped and every voter checked against the round leader — copied
        // VERBATIM. This is the only writer of header `extra_data` on a DPoS
        // chain, and both the proposer and every importer run it, so build and
        // import are identical by construction with no out-of-band map. The
        // executor decodes these same bytes for `recordProduction`.
        extra_data: order.extra_data.clone(),
        slot_number: None,
    };

    let mut builder = evm_config
        .builder_for_next_block(&mut db, &parent_sealed, attrs)
        .map_err(|e| eyre::eyre!("builder_for_next_block: {e}"))?;
    builder.apply_pre_execution_changes()?;

    for tx in &order.txs {
        let recovered = match tx.try_clone_into_recovered() {
            Ok(recovered) => recovered,
            // Deterministic skip: recovery is a pure function of the tx
            // bytes, so every node skips the same txs. Unreachable behind an
            // honest quorum (verify rejects unrecoverable signatures), kept
            // for byzantine-agreed artifacts.
            Err(error) => {
                tracing::warn!(%error, "derive: skipping unrecoverable tx");
                continue;
            }
        };
        // EXACT copy of the stock builder's skip rule
        // (ethereum/payload/src/lib.rs:370-407) minus the pool bookkeeping —
        // diverging from it would fork derived blocks between nodes.
        match builder.execute_transaction(recovered) {
            Ok(_) => {}
            Err(BlockExecutionError::Validation(BlockValidationError::InvalidTx { .. }))
            | Err(BlockExecutionError::Validation(
                BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas { .. },
            )) => continue,
            Err(fatal) => return Err(fatal.into()),
        }
    }

    let outcome = builder.finish(&state_provider, None)?;
    let state = db.take_bundle();
    Ok(DerivedExecution {
        recovered: outcome.block,
        output: BlockExecutionOutput {
            result: outcome.execution_result,
            state,
        },
        hashed_state: outcome.hashed_state,
        trie_updates: outcome.trie_updates,
        beacon_active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::{FluentEvmConfig, FluentEvmFactory};
    use alloy_consensus::{SignableTransaction as _, TxEip1559};
    use alloy_genesis::GenesisAccount;
    use alloy_primitives::{Address, Bytes, TxKind, U256};
    use alloy_signer::SignerSync as _;
    use alloy_signer_local::PrivateKeySigner;
    use fluentbase_consensus::Digest;
    use reth_chainspec::{
        make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, DEV_HARDFORKS,
    };
    use reth_db_common::init::init_genesis;
    use reth_ethereum_primitives::{Transaction, TransactionSigned};
    use reth_provider::test_utils::create_test_provider_factory_with_chain_spec;
    use std::sync::Arc;

    const DEV_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn signed_transfer(signer: &PrivateKeySigner, nonce: u64) -> TransactionSigned {
        let tx = TxEip1559 {
            chain_id: 1337,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(Address::repeat_byte(0x55)),
            value: U256::from(1u64),
            ..Default::default()
        };
        let sig = signer.sign_hash_sync(&tx.signature_hash()).expect("sign");
        TransactionSigned::new_unhashed(Transaction::Eip1559(tx), sig)
    }

    // Determinism is THE property the committee's `result` agreement rests
    // on: two independent derivations of the same (order, parent) must be
    // byte-identical, including the deterministic skip of an
    // invalid-at-its-turn tx (nonce gap).
    #[test]
    fn derive_is_deterministic_and_skips_invalid_txs() {
        let signer: PrivateKeySigner = DEV_KEY.parse().expect("key");
        let genesis = fluentbase_genesis::local_genesis_from_file().extend_accounts([(
            signer.address(),
            GenesisAccount::default().with_balance(U256::from(10u64).pow(U256::from(18u64))),
        )]);
        let hardforks = DEV_HARDFORKS.clone();
        let chain_spec: Arc<ChainSpec> = Arc::new(ChainSpec {
            chain: Chain::from(1337u64),
            genesis_header: reth_primitives_traits::SealedHeader::new_unhashed(
                make_genesis_header(&genesis, &hardforks),
            ),
            genesis,
            paris_block_and_final_difficulty: Some((0, U256::ZERO)),
            hardforks,
            base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
            deposit_contract: None,
            ..Default::default()
        });

        let factory = create_test_provider_factory_with_chain_spec(chain_spec.clone());
        let genesis_hash = init_genesis(&factory).expect("init genesis");
        let provider =
            reth_provider::providers::BlockchainProvider::new(factory).expect("provider");

        let genesis_header = chain_spec.genesis_header();
        let order = OrderBlock {
            parent: Digest(B256::ZERO),
            height: genesis_header.number + 1,
            proposal_view: 0,
            timestamp: genesis_header.timestamp + 1,
            fee_recipient: Address::repeat_byte(0x77),
            gas_limit: genesis_header.gas_limit,
            // The real 2-byte production record, not an arbitrary pair: this
            // test is the ONLY place the record's trip into an EVM header is
            // pinned. `extra_data` reaches a header by exactly one path — this
            // `order.extra_data.clone()` — because the payload registry that
            // `payload.rs` reads has no writer anywhere in the workspace, so a
            // locally built payload always ships the force-emptied base.
            extra_data: Bytes::from(fluentbase_consensus::extra_data::encode_production_record(
                42,
            )),
            result: B256::ZERO,
            txs: vec![signed_transfer(&signer, 0), signed_transfer(&signer, 7)],
            beacon_outcome: None,
            dkg_logs: Vec::new(),
            parent_seed: None,
        };

        let evm_config = FluentEvmConfig::new(
            chain_spec.clone(),
            FluentEvmFactory::default(),
            Address::ZERO,
            Address::ZERO,
            Address::ZERO,
        );

        let prev_randao = B256::repeat_byte(0x42);
        let a = derive_sync(
            &provider,
            &evm_config,
            &order,
            genesis_hash,
            prev_randao,
            None,
        )
        .expect("derive a");
        let b = derive_sync(
            &provider,
            &evm_config,
            &order,
            genesis_hash,
            prev_randao,
            None,
        )
        .expect("derive b");

        assert_eq!(
            a.evm_hash(),
            b.evm_hash(),
            "derivation must be byte-identical"
        );
        let a = a.recovered.into_sealed_block();
        // nonce-7 (gap) deterministically skipped; nonce-0 included.
        assert_eq!(a.body().transactions.len(), 1);
        // Agreed-field mapping into the derived header.
        assert_eq!(a.header().beneficiary, order.fee_recipient);
        assert_eq!(a.header().timestamp, order.timestamp);
        assert_eq!(a.header().gas_limit, order.gas_limit);
        assert_eq!(a.header().extra_data, order.extra_data);
        // Byte-verbatim is necessary but not sufficient — assert the header the
        // EXECUTOR will decode still names the same producer. A silent re-encode
        // between the OrderBlock and the header would mis-credit production with
        // no other symptom (verify passes, the state root still agrees, and only
        // the on-chain counter is wrong).
        let record = fluentbase_consensus::extra_data::decode_production_record(
            a.header().extra_data.as_ref(),
        )
        .expect("header carries a decodable record")
        .expect("and it is not the empty migration-window case");
        assert_eq!(record.leader_index, 42);
        // prev_randao is the caller-resolved value, not the ordering digest.
        assert_eq!(a.header().mix_hash, prev_randao);
    }

    // Against the CURRENT reth pin, a parent-header read landing inside the
    // static-file freeze window surfaces as a spurious `Ok(None)` (the fork's
    // nippy-jar guard turns the torn-offset panic into an error that the
    // pinned `StaticFileCursor::get` still swallows) — the belt must absorb it
    // and recover on a later attempt. Once the reth cursor fix lands, the torn
    // read arrives as `Err(InconsistentData)` (outer belt's class) and this
    // belt's `Ok(None)` arm covers only the eager-canon visibility lag.
    #[test]
    fn parent_header_retry_recovers_from_transient_none() {
        let hash = B256::repeat_byte(0x11);
        let mut calls = 0u32;
        let header = parent_header_with_transient_retry(
            || -> Result<Option<Header>, std::convert::Infallible> {
                calls += 1;
                Ok((calls > 3).then(Header::default))
            },
            hash,
            std::time::Duration::ZERO,
        )
        .expect("recovers once the segment is readable again");
        assert_eq!(header, Header::default());
        assert_eq!(calls, 4, "stops at the first successful read");
    }

    // Exhaustion must propagate EXACTLY the unretried error (ParentHeaderMissing,
    // the type the recovery/jump visibility belts downcast on) after a bounded
    // number of attempts — genuine absence/corruption stays loud and fatal.
    #[test]
    fn parent_header_retry_is_bounded_and_keeps_the_error() {
        let hash = B256::repeat_byte(0x22);
        let mut calls = 0u32;
        let err = parent_header_with_transient_retry(
            || -> Result<Option<Header>, std::convert::Infallible> {
                calls += 1;
                Ok(None)
            },
            hash,
            std::time::Duration::ZERO,
        )
        .expect_err("permanent absence must exhaust");
        assert_eq!(calls, TRANSIENT_READ_ATTEMPTS, "bounded — no retry-forever");
        let missing = err
            .downcast_ref::<ParentHeaderMissing>()
            .expect("error type unchanged from the unretried path");
        assert_eq!(missing.0, hash);
    }

    // A provider `Err` is NOT a torn static-file read (the guard surfaces those
    // as `Ok(None)`) — it must propagate immediately, without retries.
    #[test]
    fn parent_header_retry_does_not_retry_provider_errors() {
        let hash = B256::repeat_byte(0x33);
        let mut calls = 0u32;
        let err = parent_header_with_transient_retry(
            || -> Result<Option<Header>, std::io::Error> {
                calls += 1;
                Err(std::io::Error::other("mdbx exploded"))
            },
            hash,
            std::time::Duration::ZERO,
        )
        .expect_err("provider errors are immediately fatal");
        assert_eq!(calls, 1, "no retry on Err");
        assert!(err.to_string().contains("read parent header"));
    }

    // A torn static-file changeset read mid-derive (reth-fork guard: former
    // from_compact panic → ProviderError::Database(DatabaseError::Decode))
    // clears once the persistence append settles — the whole-derive belt must
    // re-run the derive and recover, even with the typed error buried under
    // eyre context (the chain-walk classification).
    #[test]
    fn changeset_decode_retry_recovers_after_transient_error() {
        use reth_storage_api::errors::{db::DatabaseError, ProviderError};
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 2 {
                    Err(
                        eyre::Report::new(ProviderError::Database(DatabaseError::Decode))
                            .wrap_err("some derive-path context"),
                    )
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("recovers once the append settles");
        assert_eq!(out, "derived");
        assert_eq!(calls, 3, "stops at the first successful derive");
    }

    // Genuine corruption surfaces as the SAME Decode error — the belt must stay
    // bounded and propagate the error unchanged after exhaustion.
    #[test]
    fn changeset_decode_retry_is_bounded_and_propagates() {
        use reth_storage_api::errors::{db::DatabaseError, ProviderError};
        let mut calls = 0u32;
        let err = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                Err(ProviderError::Database(DatabaseError::Decode).into())
            },
        )
        .expect_err("persistent decode must exhaust");
        assert_eq!(
            calls, TORN_TOTAL_ATTEMPTS,
            "bounded at the two-phase torn budget — no retry-forever"
        );
        assert!(matches!(
            err.downcast_ref::<ProviderError>(),
            Some(ProviderError::Database(DatabaseError::Decode))
        ));
    }

    // Any other derive failure is NOT the torn-read class — it must propagate
    // immediately (fatal path unchanged, no retry).
    #[test]
    fn changeset_decode_retry_ignores_other_errors() {
        let mut calls = 0u32;
        let err = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                Err(eyre::eyre!("state root mismatch"))
            },
        )
        .expect_err("non-decode errors are immediately fatal");
        assert_eq!(calls, 1, "no retry on unclassified errors");
        assert!(err.to_string().contains("state root mismatch"));
    }

    /// The mdbx read-tx timeout as it reaches the fluentbase boundary: the
    /// per-call-site variant (`Read` on the cursor paths of the changeset
    /// walk) carrying `DatabaseErrorInfo { message, code: -96000 }`.
    fn read_tx_timeout_error() -> reth_storage_api::errors::ProviderError {
        use reth_storage_api::errors::db::{DatabaseError, DatabaseErrorInfo};
        reth_storage_api::errors::ProviderError::Database(DatabaseError::Read(DatabaseErrorInfo {
            message: "read transaction has been timed out".into(),
            code: MDBX_READ_TX_TIMEOUT_CODE,
        }))
    }

    // Cold-catch-up convergent class: the changeset walk exceeded the 300 s
    // read-txn limit; a fresh txn resumes against the per-block-warmed
    // changeset cache, so retrying converges — the belt must absorb a
    // timeout-then-success sequence.
    #[test]
    fn db_timeout_retry_recovers_after_transient_timeouts() {
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 7 {
                    Err(eyre::Report::new(read_tx_timeout_error())
                        .wrap_err("derive_and_execute failed"))
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("converges once the cache is warm enough");
        assert_eq!(out, "derived");
        assert_eq!(
            calls, 8,
            "more than the torn-read budget — the timeout class has its own"
        );
    }

    // A persistent timeout exhausts at the CLASS budget (64), not at the
    // torn-read budget (5), and propagates the error unchanged.
    #[test]
    fn db_timeout_retry_exhausts_at_its_own_budget() {
        use reth_storage_api::errors::ProviderError;
        let mut calls = 0u32;
        let err = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                Err(read_tx_timeout_error().into())
            },
        )
        .expect_err("persistent timeout must exhaust");
        assert_eq!(
            calls, CONVERGENT_DB_TIMEOUT_ATTEMPTS,
            "bounded at the timeout-class budget"
        );
        assert!(matches!(
            err.downcast_ref::<ProviderError>(),
            Some(ProviderError::Database(_))
        ));
    }

    // The class budgets must not cross-contaminate: a torn-read error stream
    // still exhausts at TORN_TOTAL_ATTEMPTS even though the timeout class
    // would allow far more, and interleaved timeouts don't reset the torn
    // counter.
    #[test]
    fn class_budgets_do_not_cross_contaminate() {
        use reth_storage_api::errors::{db::DatabaseError, ProviderError};
        // Pure torn-read stream: exhausts at the two-phase torn budget (15),
        // independent of the timeout class.
        let mut torn_calls = 0u32;
        let _ = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                torn_calls += 1;
                Err(ProviderError::Database(DatabaseError::Decode).into())
            },
        )
        .expect_err("torn class exhausts");
        assert_eq!(torn_calls, TORN_TOTAL_ATTEMPTS);
        // Interleaved: 3 timeouts then torn-reads — the torn class still gets
        // its full own budget (3 + 15 calls), the timeouts consumed none of it.
        let mut calls = 0u32;
        let _ = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                if calls <= 3 {
                    Err(read_tx_timeout_error().into())
                } else {
                    Err(ProviderError::Database(DatabaseError::Decode).into())
                }
            },
        )
        .expect_err("torn class exhausts after interleaved timeouts");
        assert_eq!(calls, 3 + TORN_TOTAL_ATTEMPTS);
    }

    // Classifier precision: the stable display matches (incl. through
    // stringifying wrappers), but write-transaction failures and the
    // NEIGHBORING custom code -96001 (BotchedTransaction) must not.
    #[test]
    fn db_timeout_classifier_matches_typed_display_and_rejects_lookalikes() {
        use reth_storage_api::errors::db::{DatabaseError, DatabaseErrorInfo};
        use reth_storage_api::errors::ProviderError;
        // Typed under eyre context.
        let typed = eyre::Report::new(read_tx_timeout_error()).wrap_err("derive context");
        assert!(is_convergent_db_timeout(&typed));
        // Stringified wrapper — display fallback.
        let stringified = eyre::eyre!("derive_and_execute failed: {}", read_tx_timeout_error());
        assert!(is_convergent_db_timeout(&stringified));
        // Lookalike rejections.
        assert!(!is_convergent_db_timeout(&eyre::eyre!(
            "write transaction has been timed out"
        )));
        let botched = ProviderError::Database(DatabaseError::Read(DatabaseErrorInfo {
            message: "transaction is botched".into(),
            code: -96001,
        }));
        assert!(!is_convergent_db_timeout(&eyre::Report::new(botched)));
        // And the timeout is NOT the torn-read class (disjoint classifiers).
        assert!(!is_transient_torn_static_file_read(&eyre::Report::new(
            read_tx_timeout_error()
        )));
    }

    // Second torn-read manifestation: the reth-fork cursor fix propagates
    // `NippyJarError::InconsistentData` as `ProviderError::Other(AnyError)`.
    // `AnyError::source()` skips the wrapped error as a chain link (and
    // fluentbase has no reth-nippy-jar dep), so the classifier matches the
    // typed `Other` variant by its payload display. A same-display error type
    // stands in for NippyJarError here — the classifier never sees more than
    // the variant + display either way.
    #[test]
    fn torn_range_classifier_matches_provider_other_typed_and_stringified() {
        use reth_storage_api::errors::ProviderError;
        let torn = || {
            ProviderError::other(std::io::Error::other(
                "attempted to read an inconsistent data range 10..2, data size: 8",
            ))
        };
        // Typed: ProviderError::Other survives the eyre chain under context.
        let typed = eyre::Report::new(torn()).wrap_err("some derive-path context");
        assert!(is_transient_torn_static_file_read(&typed));
        // Stringified wrapper (eyre!("...: {e}")) — display fallback.
        let stringified = eyre::eyre!("builder_for_next_block: {}", torn());
        assert!(is_transient_torn_static_file_read(&stringified));
        // Lookalikes: a different ProviderError::Other payload, and a bare
        // "inconsistent data" phrase, are NOT the torn-range class.
        let other_payload = eyre::Report::new(ProviderError::other(std::io::Error::other(
            "some unrelated provider failure",
        )));
        assert!(!is_transient_torn_static_file_read(&other_payload));
        assert!(!is_transient_torn_static_file_read(&eyre::eyre!(
            "inconsistent data detected in trie walker"
        )));
    }

    // Third torn-read manifestation: a SHORT READ of a static-file sidecar
    // (the changeset-offsets `read_exact_at` reading a still-appending offsets
    // file) surfaces as a `std::io::Error` of kind `UnexpectedEof` (display
    // "failed to fill whole buffer") buried under eyre context at `finish`. The
    // append settles sub-second, so the whole-derive belt must re-run and
    // recover with the typed io error preserved in the chain.
    #[test]
    fn short_read_retry_recovers_from_typed_unexpected_eof() {
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 2 {
                    Err(eyre::Report::new(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "failed to fill whole buffer",
                    ))
                    .wrap_err("derive_and_execute failed"))
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("recovers once the sidecar append settles");
        assert_eq!(out, "derived");
        assert_eq!(calls, 3, "stops at the first successful derive");
    }

    // A boundary wrapper that stringifies the io error (`eyre!("...: {e}")`)
    // erases the typed `io::Error` chain link — the classifier's display
    // fallback must still catch the stable short-read display and recover.
    #[test]
    fn short_read_retry_recovers_from_stringified_wrapper() {
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 1 {
                    let io = std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "failed to fill whole buffer",
                    );
                    Err(eyre::eyre!("builder.finish: {io}"))
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("display fallback recovers");
        assert_eq!(out, "derived");
        assert_eq!(calls, 2);
    }

    // Classifier precision: typed `UnexpectedEof` and the stringified display
    // both match, but a DIFFERENT io `ErrorKind` with an unrelated display must
    // NOT (a genuine non-torn io failure stays fatal, unretried).
    #[test]
    fn short_read_classifier_matches_eof_and_rejects_lookalikes() {
        // Typed UnexpectedEof under eyre context — the changeset-offsets short read.
        let typed = eyre::Report::new(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "failed to fill whole buffer",
        ))
        .wrap_err("derive_and_execute failed");
        assert!(is_transient_torn_static_file_read(&typed));
        // Stringified wrapper — display fallback.
        assert!(is_transient_torn_static_file_read(&eyre::eyre!(
            "builder.finish: failed to fill whole buffer"
        )));
        // Lookalike: a different io ErrorKind with an unrelated display is NOT
        // the torn/short-read class.
        let lookalike = eyre::Report::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "mdbx permission denied",
        ));
        assert!(!is_transient_torn_static_file_read(&lookalike));
        // And a short read is NOT the convergent-timeout class (disjoint).
        assert!(!is_convergent_db_timeout(&typed));
    }

    // Genuine corruption raises the SAME UnexpectedEof — the belt must stay
    // bounded at the torn-read budget and propagate the io error unchanged.
    #[test]
    fn short_read_retry_is_bounded_and_propagates() {
        let mut calls = 0u32;
        let err = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                Err(eyre::Report::new(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                )))
            },
        )
        .expect_err("persistent short read must exhaust");
        assert_eq!(
            calls, TORN_TOTAL_ATTEMPTS,
            "bounded at the two-phase torn budget"
        );
        assert!(err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof));
    }

    // Fourth torn manifestation: the reth-fork committed-snapshot reader
    // discipline refuses a self-load of an active writer file / fails the
    // `LoadedJar` reload-once validation → `ProviderError::Other` over
    // `NippyJarError::InconsistentSnapshot` (display "inconsistent static-file
    // snapshot: {reason}"). Transient (mark→publish window / mid-heal read) —
    // the belt must re-run the derive and recover with the typed
    // `ProviderError::Other` preserved under eyre context.
    #[test]
    fn snapshot_retry_recovers_from_typed_provider_other() {
        use reth_storage_api::errors::ProviderError;
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 2 {
                    Err(
                        eyre::Report::new(ProviderError::other(std::io::Error::other(
                            "inconsistent static-file snapshot: active writer file refused",
                        )))
                        .wrap_err("derive_and_execute failed"),
                    )
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("recovers once the snapshot publishes/heals");
        assert_eq!(out, "derived");
        assert_eq!(calls, 3, "stops at the first successful derive");
    }

    // A boundary wrapper that stringifies the source (`eyre!("...: {e}")`)
    // erases the typed chain — the classifier's display fallback must still
    // catch the stable snapshot display and recover.
    #[test]
    fn snapshot_retry_recovers_from_stringified_wrapper() {
        use reth_storage_api::errors::ProviderError;
        let mut calls = 0u32;
        let out = with_transient_derive_retry(
            42,
            |_| {},
            || {
                calls += 1;
                if calls <= 1 {
                    let inner = ProviderError::other(std::io::Error::other(
                        "inconsistent static-file snapshot: reload-once validation failed",
                    ));
                    Err(eyre::eyre!("builder.finish: {inner}"))
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("display fallback recovers");
        assert_eq!(out, "derived");
        assert_eq!(calls, 2);
    }

    // Classifier precision for the snapshot shape: typed ProviderError::Other
    // and the stringified display match; the existing rejection corpus still
    // holds with the new prefix (a bare "inconsistent data ..." phrase and an
    // unrelated Other payload must NOT match), and the shape is disjoint from
    // the timeout class.
    #[test]
    fn snapshot_classifier_matches_and_rejects_lookalikes() {
        use reth_storage_api::errors::ProviderError;
        let snap = || {
            ProviderError::other(std::io::Error::other(
                "inconsistent static-file snapshot: active writer file refused",
            ))
        };
        let typed = eyre::Report::new(snap()).wrap_err("some derive-path context");
        assert!(is_transient_torn_static_file_read(&typed));
        assert!(is_transient_torn_static_file_read(&eyre::eyre!(
            "derive_and_execute failed: {}",
            snap()
        )));
        // Rejection corpus unchanged by the new prefix: neither the trie-walker
        // phrase nor an unrelated Other payload matches.
        assert!(!is_transient_torn_static_file_read(&eyre::eyre!(
            "inconsistent data detected in trie walker"
        )));
        let other_payload = eyre::Report::new(ProviderError::other(std::io::Error::other(
            "some unrelated provider failure",
        )));
        assert!(!is_transient_torn_static_file_read(&other_payload));
        // Disjoint from the convergent-timeout class.
        assert!(!is_convergent_db_timeout(&typed));
    }

    // Genuine PERSISTENT snapshot inconsistency raises the same error every
    // attempt — the belt must exhaust at the two-phase torn budget and
    // propagate the last error unchanged (fatal path untouched).
    #[test]
    fn snapshot_retry_is_bounded_and_propagates() {
        use reth_storage_api::errors::ProviderError;
        let mut calls = 0u32;
        let err = with_transient_derive_retry(
            42,
            |_| {},
            || -> eyre::Result<()> {
                calls += 1;
                Err(ProviderError::other(std::io::Error::other(
                    "inconsistent static-file snapshot: active writer file refused",
                ))
                .into())
            },
        )
        .expect_err("persistent snapshot inconsistency must exhaust");
        assert_eq!(
            calls, TORN_TOTAL_ATTEMPTS,
            "bounded at the two-phase torn budget"
        );
        assert!(matches!(
            err.downcast_ref::<ProviderError>(),
            Some(ProviderError::Other(_))
        ));
    }

    // bundle-20260717T074614Z (validator-2 death): the torn RANGE manifestation
    // on a ~152 MB static-file segment surfaced as an INVERTED range read
    // `attempted to read an inconsistent data range 152247225..0, data size:
    // 152247702` (ProviderError::Other over NippyJarError::InconsistentData),
    // buried under the executor's `.wrap_err("derive_and_execute failed")`. It
    // MUST match the classifier — proving verdict (a): the node died because the
    // then-single-phase 125 ms belt EXHAUSTED mid-append on a large segment, NOT
    // because the classifier missed (verdict (b)). The two-phase budget is the fix.
    #[test]
    fn bundle_20260717_torn_range_matches_classifier() {
        use reth_storage_api::errors::ProviderError;
        let torn = || {
            ProviderError::other(std::io::Error::other(
                "attempted to read an inconsistent data range 152247225..0, \
                 data size: 152247702",
            ))
        };
        // Exactly as derive_sync's `?` at `builder.finish` + the executor's
        // outer `.wrap_err("derive_and_execute failed")` produce it.
        let report = eyre::Report::new(torn()).wrap_err("derive_and_execute failed");
        assert!(
            is_transient_torn_static_file_read(&report),
            "the bundle death error IS the torn class — the fast belt exhausted mid-append"
        );
        // And it is NOT the convergent-timeout class (disjoint classifiers).
        assert!(!is_convergent_db_timeout(&report));
    }

    // Two-phase torn budget: the fast phase issues TRANSIENT_READ_ATTEMPTS
    // backoffs of TRANSIENT_READ_BACKOFF, then the slow phase escalates to
    // TORN_SLOW_BACKOFF until exhaustion at TORN_TOTAL_ATTEMPTS. A recording
    // sleep seam observes the schedule without real waiting; recovery in the
    // SLOW phase (after the fast phase already exhausted) still succeeds, and a
    // persistent stream exhausts at exactly the two-phase budget.
    #[test]
    fn torn_two_phase_backoff_schedule_and_slow_recovery() {
        use reth_storage_api::errors::{db::DatabaseError, ProviderError};
        // Recovery in the slow phase: fail through the whole fast phase and into
        // the slow phase, then succeed on the 8th call.
        let recover_at = TRANSIENT_READ_ATTEMPTS + 3;
        let mut calls = 0u32;
        let mut waits: Vec<std::time::Duration> = Vec::new();
        let out = with_transient_derive_retry(
            42,
            |d| waits.push(d),
            || {
                calls += 1;
                if calls < recover_at {
                    Err(ProviderError::Database(DatabaseError::Decode).into())
                } else {
                    Ok("derived")
                }
            },
        )
        .expect("recovers in the slow phase after the fast phase exhausts");
        assert_eq!(out, "derived");
        assert_eq!(calls, recover_at);
        // One backoff per FAILED attempt (recover_at - 1): first
        // TRANSIENT_READ_ATTEMPTS fast, the remainder slow.
        assert_eq!(waits.len() as u32, recover_at - 1);
        for (i, w) in waits.iter().enumerate() {
            let expected = if (i as u32) < TRANSIENT_READ_ATTEMPTS {
                TRANSIENT_READ_BACKOFF
            } else {
                TORN_SLOW_BACKOFF
            };
            assert_eq!(*w, expected, "backoff #{i} is phase-correct");
        }

        // Full exhaust at TORN_TOTAL_ATTEMPTS (15) calls with exactly
        // TORN_TOTAL_ATTEMPTS-1 backoffs (no sleep on the exhausting attempt):
        // TRANSIENT_READ_ATTEMPTS fast + (TORN_SLOW_ATTEMPTS-1) slow.
        let mut calls = 0u32;
        let mut waits: Vec<std::time::Duration> = Vec::new();
        let _ = with_transient_derive_retry(
            42,
            |d| waits.push(d),
            || -> eyre::Result<()> {
                calls += 1;
                Err(ProviderError::Database(DatabaseError::Decode).into())
            },
        )
        .expect_err("persistent torn read exhausts at the two-phase budget");
        assert_eq!(calls, TORN_TOTAL_ATTEMPTS);
        assert_eq!(waits.len() as u32, TORN_TOTAL_ATTEMPTS - 1);
        let fast = waits
            .iter()
            .filter(|w| **w == TRANSIENT_READ_BACKOFF)
            .count() as u32;
        let slow = waits.iter().filter(|w| **w == TORN_SLOW_BACKOFF).count() as u32;
        assert_eq!(fast, TRANSIENT_READ_ATTEMPTS, "fast-phase backoff count");
        assert_eq!(slow, TORN_SLOW_ATTEMPTS - 1, "slow-phase backoff count");
    }

    // Boundary wrappers that stringify the source (InternalBlockExecutionError's
    // `{error}` display, `eyre!("...: {e}")`) erase the typed chain — the
    // classifier's display fallback must still catch the stable Decode display.
    #[test]
    fn changeset_decode_classifier_matches_stringified_wrappers() {
        use reth_storage_api::errors::{db::DatabaseError, ProviderError};
        let stringified = eyre::eyre!(
            "builder_for_next_block: {}",
            ProviderError::Database(DatabaseError::Decode)
        );
        assert!(is_transient_torn_static_file_read(&stringified));
        assert!(!is_transient_torn_static_file_read(&eyre::eyre!(
            "failed to decode something unrelated"
        )));
    }

    // The derive-gate (Decision C4): with a beacon source whose cache holds a
    // verifiable seed(h) for the target height, prev_randao(h) = H(seed(h));
    // otherwise (height absent, or no beacon source) it degrades to the weak
    // deterministic fallback — never blocking, the Q4 assurance=false path.
    #[test]
    fn resolve_prev_randao_uses_seed_else_falls_back() {
        use commonware_consensus::types::{Epoch, Round, View};
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use fluentbase_bls::beacon::{recover_seed, sign_seed_partial};
        use fluentbase_consensus::beacon::seed::{prev_randao_from_seed, seed_namespace};

        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        let ns = seed_namespace(b"fluent-devnet");
        let round = Round::new(Epoch::new(1), View::new(50));
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        let seed = Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover"),
        };
        let expected = prev_randao_from_seed(&seed);
        let fallback = B256::repeat_byte(0x99);

        // Seed present → threshold randomness `H(seed)`, beacon_active = Some(true).
        // The seed is bound by the multisig cert, so the deriver trusts it — no
        // on-chain PK_E re-verify (that layer is gone, DPOS_ARCHITECTURE §8.11).
        assert_eq!(
            resolve_prev_randao(Some(&seed), fallback),
            (expected, Some(true))
        );
        // No seed → fallback, not a beacon observation.
        assert_eq!(resolve_prev_randao(None, fallback), (fallback, None));
    }
}
