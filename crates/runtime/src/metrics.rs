//! Prometheus metrics emitted by the runtime executor.
//!
//! The node installs the Prometheus recorder via `reth-node-metrics`; this crate only records
//! low-cardinality runtime observations into that global recorder.
//!
//! Everything on the execute/resume path records through handles that are registered once per
//! label set and reused afterwards. Building a `metrics::Key` with owned labels and resolving it
//! against the recorder on every runtime call cost about a fifth of an interruption round trip.
//! A handle binds to the recorder installed when it is first used, so the recorder has to be in
//! place before the first execution; the node installs it during launch, ahead of genesis
//! initialization and of any block or RPC execution.
#![cfg_attr(not(feature = "std"), allow(unused_variables))]

use crate::executor::ExecutionResult;
use fluentbase_types::{
    CompilationBackend, CompilationConfigFingerprint, ExitCode,
    COMPILATION_CONFIG_FINGERPRINT_VERSION, STATE_DEPLOY, STATE_MAIN,
};
use rwasm::TrapCode;

#[cfg(feature = "std")]
use metrics::{Counter, Gauge, Histogram};
#[cfg(feature = "std")]
use std::{sync::OnceLock, time::Instant};

/// Low-cardinality runtime mode label.
#[derive(Clone, Copy, Debug)]
pub enum RuntimeModeLabel {
    Contract,
    System,
}

impl RuntimeModeLabel {
    const COUNT: usize = 2;

    fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::System => "system",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Contract => 0,
            Self::System => 1,
        }
    }
}

/// Runtime entrypoint/state label.
#[derive(Clone, Copy, Debug)]
pub enum RuntimeStateLabel {
    Main,
    Deploy,
    Unknown,
}

impl RuntimeStateLabel {
    const COUNT: usize = 3;

    pub fn from_state(state: u32) -> Self {
        match state {
            STATE_MAIN => Self::Main,
            STATE_DEPLOY => Self::Deploy,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Deploy => "deploy",
            Self::Unknown => "unknown",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Deploy => 1,
            Self::Unknown => 2,
        }
    }
}

/// Which executor entry point produced an observation.
#[derive(Clone, Copy, Debug)]
enum Operation {
    Execution,
    Resume,
}

impl Operation {
    const COUNT: usize = 2;

    fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Resume => "resume",
        }
    }

    fn total_metric(self) -> &'static str {
        match self {
            Self::Execution => "fluentbase_runtime_executions_total",
            Self::Resume => "fluentbase_runtime_resumes_total",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Execution => 0,
            Self::Resume => 1,
        }
    }
}

/// Coarse classification of an execution result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Interrupted,
    Success,
    OutOfFuel,
    Fatal,
    Reverted,
}

impl Outcome {
    const COUNT: usize = 5;

    fn from_result(result: &ExecutionResult) -> Self {
        if result.exit_code > 0 {
            Self::Interrupted
        } else if result.exit_code == ExitCode::Ok.into_i32() {
            Self::Success
        } else if result.exit_code == ExitCode::OutOfFuel.into_i32() {
            Self::OutOfFuel
        } else if result.exit_code == ExitCode::UnexpectedFatalExecutionFailure.into_i32() {
            Self::Fatal
        } else {
            Self::Reverted
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Interrupted => "interrupted",
            Self::Success => "success",
            Self::OutOfFuel => "out_of_fuel",
            Self::Fatal => "fatal",
            Self::Reverted => "reverted",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Interrupted => 0,
            Self::Success => 1,
            Self::OutOfFuel => 2,
            Self::Fatal => 3,
            Self::Reverted => 4,
        }
    }
}

/// Execution timer. In non-std builds metrics are no-ops.
#[derive(Debug)]
pub struct RuntimeTimer {
    #[cfg(feature = "std")]
    started_at: Instant,
}

impl RuntimeTimer {
    pub fn start() -> Self {
        Self {
            #[cfg(feature = "std")]
            started_at: Instant::now(),
        }
    }

    #[cfg(feature = "std")]
    fn elapsed_seconds(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }
}

/// Handles for every observation recorded about one execute or resume call.
#[cfg(feature = "std")]
struct OperationMetrics {
    execution_seconds: Histogram,
    fuel_consumed: Histogram,
    fuel_refunded: Histogram,
    output_bytes: Histogram,
    return_data_bytes: Histogram,
    /// `fluentbase_runtime_executions_total` or `fluentbase_runtime_resumes_total`.
    total: Counter,
    /// `fluentbase_runtime_interruptions_total`, present only for interrupted outcomes.
    interruptions: Option<Counter>,
}

#[cfg(feature = "std")]
const OPERATION_CELLS: usize =
    Operation::COUNT * RuntimeModeLabel::COUNT * RuntimeStateLabel::COUNT * Outcome::COUNT;

#[cfg(feature = "std")]
static OPERATION_METRICS: [OnceLock<OperationMetrics>; OPERATION_CELLS] =
    [const { OnceLock::new() }; OPERATION_CELLS];

#[cfg(feature = "std")]
fn operation_cell(
    operation: Operation,
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    outcome: Outcome,
) -> usize {
    ((operation.index() * RuntimeModeLabel::COUNT + mode.index()) * RuntimeStateLabel::COUNT
        + state.index())
        * Outcome::COUNT
        + outcome.index()
}

#[cfg(feature = "std")]
fn register_operation_metrics(
    operation: Operation,
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    outcome: Outcome,
) -> OperationMetrics {
    let labels = [
        ("operation", operation.as_str()),
        ("mode", mode.as_str()),
        ("state", state.as_str()),
        ("outcome", outcome.as_str()),
    ];
    OperationMetrics {
        execution_seconds: metrics::histogram!("fluentbase_runtime_execution_seconds", &labels),
        fuel_consumed: metrics::histogram!("fluentbase_runtime_fuel_consumed", &labels),
        fuel_refunded: metrics::histogram!("fluentbase_runtime_fuel_refunded", &labels),
        output_bytes: metrics::histogram!("fluentbase_runtime_output_bytes", &labels),
        return_data_bytes: metrics::histogram!("fluentbase_runtime_return_data_bytes", &labels),
        total: metrics::counter!(
            operation.total_metric(),
            "mode" => mode.as_str(),
            "state" => state.as_str(),
            "outcome" => outcome.as_str(),
        ),
        interruptions: (outcome == Outcome::Interrupted).then(|| {
            metrics::counter!(
                "fluentbase_runtime_interruptions_total",
                "mode" => mode.as_str(),
                "state" => state.as_str(),
            )
        }),
    }
}

#[cfg(feature = "std")]
fn operation_metrics(
    operation: Operation,
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    outcome: Outcome,
) -> &'static OperationMetrics {
    OPERATION_METRICS[operation_cell(operation, mode, state, outcome)]
        .get_or_init(|| register_operation_metrics(operation, mode, state, outcome))
}

#[cfg(feature = "std")]
fn record_operation(
    operation: Operation,
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    let metrics = operation_metrics(operation, mode, state, Outcome::from_result(result));
    metrics.execution_seconds.record(timer.elapsed_seconds());
    metrics.fuel_consumed.record(result.fuel_consumed as f64);
    metrics
        .fuel_refunded
        .record(result.fuel_refunded.max(0) as f64);
    metrics.output_bytes.record(result.output.len() as f64);
    metrics
        .return_data_bytes
        .record(result.return_data.len() as f64);
    metrics.total.increment(1);
    if let Some(interruptions) = &metrics.interruptions {
        interruptions.increment(1);
    }
}

/// Records an execution that failed before the runtime could be constructed.
pub fn record_initialization_error(
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    trap: TrapCode,
) {
    #[cfg(feature = "std")]
    {
        metrics::counter!(
            "fluentbase_runtime_initialization_errors_total",
            "mode" => mode.as_str(),
            "state" => state.as_str(),
            "trap" => trap_label(trap),
        )
        .increment(1);
    }
}

/// Records one fresh runtime execution.
pub fn record_execution(
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    #[cfg(feature = "std")]
    {
        set_compilation_cache_fingerprint_version();
        record_operation(Operation::Execution, mode, state, timer, result);
    }
}

/// Records one resume attempt.
pub fn record_resume(
    mode: RuntimeModeLabel,
    state: RuntimeStateLabel,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    #[cfg(feature = "std")]
    record_operation(Operation::Resume, mode, state, timer, result);
}

#[cfg(feature = "std")]
static FORGOTTEN_RUNTIMES: [OnceLock<Counter>; RuntimeModeLabel::COUNT * RuntimeStateLabel::COUNT] =
    [const { OnceLock::new() }; RuntimeModeLabel::COUNT * RuntimeStateLabel::COUNT];

/// Records that a suspended runtime was explicitly dropped.
pub fn record_forget_runtime(mode: RuntimeModeLabel, state: RuntimeStateLabel) {
    #[cfg(feature = "std")]
    FORGOTTEN_RUNTIMES[mode.index() * RuntimeStateLabel::COUNT + state.index()]
        .get_or_init(|| {
            metrics::counter!(
                "fluentbase_runtime_forgotten_total",
                "mode" => mode.as_str(),
                "state" => state.as_str(),
            )
        })
        .increment(1);
}

#[cfg(feature = "std")]
static RECOVERABLE_RUNTIMES: OnceLock<Gauge> = OnceLock::new();

/// Publishes the current number of suspended runtimes held by the executor.
pub fn set_recoverable_runtimes(count: usize) {
    #[cfg(feature = "std")]
    RECOVERABLE_RUNTIMES
        .get_or_init(|| metrics::gauge!("fluentbase_runtime_recoverable_runtimes"))
        .set(count as f64);
}

#[cfg(feature = "std")]
fn backend_index(backend: CompilationBackend) -> usize {
    match backend {
        CompilationBackend::Rwasm => 0,
        CompilationBackend::Wasmtime => 1,
    }
}

#[cfg(feature = "std")]
static SYSTEM_RUNTIME_CACHE_LOOKUPS: [OnceLock<Counter>; 4] = [const { OnceLock::new() }; 4];

#[cfg(feature = "std")]
fn system_runtime_cache_lookup_counter(
    fingerprint: CompilationConfigFingerprint,
    result: &'static str,
) -> Counter {
    metrics::counter!(
        "fluentbase_system_runtime_cache_lookups_total",
        "fingerprint_version" => fingerprint.version.to_string(),
        "backend" => fingerprint.backend.as_str(),
        "result" => result,
    )
}

/// Records whether a system runtime instance cache lookup reused an exact compatible key.
pub fn record_system_runtime_cache_lookup(
    fingerprint: CompilationConfigFingerprint,
    cache_hit: bool,
) {
    #[cfg(feature = "std")]
    {
        let result = if cache_hit { "hit" } else { "miss" };
        // Every fingerprint built by `from_config` carries the current version, which makes the
        // handle cacheable by backend and result. Anything else keeps its exact labels.
        if fingerprint.version == COMPILATION_CONFIG_FINGERPRINT_VERSION {
            SYSTEM_RUNTIME_CACHE_LOOKUPS
                [backend_index(fingerprint.backend) * 2 + cache_hit as usize]
                .get_or_init(|| system_runtime_cache_lookup_counter(fingerprint, result))
                .increment(1);
        } else {
            system_runtime_cache_lookup_counter(fingerprint, result).increment(1);
        }
    }
}

/// Records deterministic invalidation of a cached system runtime instance.
pub fn record_system_runtime_cache_invalidation(
    fingerprint: CompilationConfigFingerprint,
    reason: &'static str,
) {
    #[cfg(feature = "std")]
    metrics::counter!(
        "fluentbase_system_runtime_cache_invalidations_total",
        "fingerprint_version" => fingerprint.version.to_string(),
        "backend" => fingerprint.backend.as_str(),
        "reason" => reason,
    )
    .increment(1);
}

/// Records that the compiled-module cache was discarded after a panic poisoned its lock.
pub fn record_module_cache_reset() {
    #[cfg(feature = "std")]
    metrics::counter!("fluentbase_module_cache_resets_total").increment(1);
}

/// Records a hash-only module lookup that found no cached module.
pub fn record_module_cache_hash_miss(reason: &'static str) {
    #[cfg(feature = "std")]
    metrics::counter!(
        "fluentbase_module_cache_hash_misses_total",
        "reason" => reason,
    )
    .increment(1);
}

#[cfg(feature = "std")]
static COMPILATION_CACHE_FINGERPRINT_VERSION: OnceLock<Gauge> = OnceLock::new();

/// Exposes the active cache-key version for operational dashboards.
pub fn set_compilation_cache_fingerprint_version() {
    #[cfg(feature = "std")]
    COMPILATION_CACHE_FINGERPRINT_VERSION
        .get_or_init(|| metrics::gauge!("fluentbase_compilation_cache_fingerprint_version"))
        .set(COMPILATION_CONFIG_FINGERPRINT_VERSION as f64);
}

fn trap_label(trap: TrapCode) -> &'static str {
    match trap {
        TrapCode::OutOfFuel => "out_of_fuel",
        TrapCode::InterruptionCalled => "interruption_called",
        TrapCode::MemoryOutOfBounds => "memory_out_of_bounds",
        TrapCode::StackOverflow => "stack_overflow",
        TrapCode::UnreachableCodeReached => "unreachable_code_reached",
        _ => "other",
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use metrics::{Key, KeyName, Metadata, Recorder, SharedString, Unit};
    use std::{collections::BTreeSet, sync::Mutex};

    const OPERATIONS: [Operation; Operation::COUNT] = [Operation::Execution, Operation::Resume];
    const MODES: [RuntimeModeLabel; RuntimeModeLabel::COUNT] =
        [RuntimeModeLabel::Contract, RuntimeModeLabel::System];
    const STATES: [RuntimeStateLabel; RuntimeStateLabel::COUNT] = [
        RuntimeStateLabel::Main,
        RuntimeStateLabel::Deploy,
        RuntimeStateLabel::Unknown,
    ];
    const OUTCOMES: [Outcome; Outcome::COUNT] = [
        Outcome::Interrupted,
        Outcome::Success,
        Outcome::OutOfFuel,
        Outcome::Fatal,
        Outcome::Reverted,
    ];

    /// Captures every key registered through it as `kind name{label=value,...}`.
    #[derive(Default)]
    struct CaptureRecorder {
        keys: Mutex<Vec<String>>,
    }

    impl CaptureRecorder {
        fn capture(&self, kind: &str, key: &Key) {
            let labels = key
                .labels()
                .map(|label| format!("{}={}", label.key(), label.value()))
                .collect::<Vec<_>>()
                .join(",");
            self.keys
                .lock()
                .unwrap()
                .push(format!("{kind} {}{{{labels}}}", key.name()));
        }
    }

    impl Recorder for CaptureRecorder {
        fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
            self.capture("counter", key);
            Counter::noop()
        }
        fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
            self.capture("gauge", key);
            Gauge::noop()
        }
        fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
            self.capture("histogram", key);
            Histogram::noop()
        }
    }

    #[test]
    fn every_label_combination_maps_to_a_distinct_cell() {
        let mut cells = BTreeSet::new();
        for operation in OPERATIONS {
            for mode in MODES {
                for state in STATES {
                    for outcome in OUTCOMES {
                        let cell = operation_cell(operation, mode, state, outcome);
                        assert!(
                            cell < OPERATION_CELLS,
                            "{operation:?} {mode:?} {state:?} {outcome:?}"
                        );
                        assert!(cells.insert(cell), "cell {cell} reused");
                    }
                }
            }
        }
        assert_eq!(cells.len(), OPERATION_CELLS);
    }

    #[test]
    fn outcome_follows_the_exit_code() {
        let result = |exit_code: i32| ExecutionResult {
            exit_code,
            ..Default::default()
        };
        assert_eq!(Outcome::from_result(&result(7)), Outcome::Interrupted);
        assert_eq!(Outcome::from_result(&result(0)), Outcome::Success);
        assert_eq!(
            Outcome::from_result(&result(ExitCode::OutOfFuel.into_i32())),
            Outcome::OutOfFuel
        );
        assert_eq!(
            Outcome::from_result(&result(
                ExitCode::UnexpectedFatalExecutionFailure.into_i32()
            )),
            Outcome::Fatal
        );
        assert_eq!(
            Outcome::from_result(&result(ExitCode::Panic.into_i32())),
            Outcome::Reverted
        );
    }

    #[test]
    fn registered_keys_keep_the_published_names_and_labels() {
        let recorder = CaptureRecorder::default();
        metrics::with_local_recorder(&recorder, || {
            register_operation_metrics(
                Operation::Resume,
                RuntimeModeLabel::System,
                RuntimeStateLabel::Main,
                Outcome::Interrupted,
            );
        });
        let labels = "operation=resume,mode=system,state=main,outcome=interrupted";
        let expected = [
            format!("histogram fluentbase_runtime_execution_seconds{{{labels}}}"),
            format!("histogram fluentbase_runtime_fuel_consumed{{{labels}}}"),
            format!("histogram fluentbase_runtime_fuel_refunded{{{labels}}}"),
            format!("histogram fluentbase_runtime_output_bytes{{{labels}}}"),
            format!("histogram fluentbase_runtime_return_data_bytes{{{labels}}}"),
            "counter fluentbase_runtime_resumes_total{mode=system,state=main,outcome=interrupted}"
                .to_string(),
            "counter fluentbase_runtime_interruptions_total{mode=system,state=main}".to_string(),
        ];
        assert_eq!(*recorder.keys.lock().unwrap(), expected);
    }

    #[test]
    fn successful_executions_register_no_interruption_counter() {
        let recorder = CaptureRecorder::default();
        let registered = metrics::with_local_recorder(&recorder, || {
            register_operation_metrics(
                Operation::Execution,
                RuntimeModeLabel::Contract,
                RuntimeStateLabel::Deploy,
                Outcome::Success,
            )
        });
        assert!(registered.interruptions.is_none());
        let keys = recorder.keys.lock().unwrap();
        assert_eq!(keys.len(), 6);
        assert_eq!(
            keys[5],
            "counter fluentbase_runtime_executions_total{mode=contract,state=deploy,outcome=success}"
        );
    }
}
