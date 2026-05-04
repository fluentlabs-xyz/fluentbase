//! Prometheus metrics emitted by the runtime executor.
//!
//! The node installs the Prometheus recorder via `reth-node-metrics`; this crate only records
//! low-cardinality runtime observations into that global recorder.
#![cfg_attr(not(feature = "std"), allow(unused_variables))]

use crate::executor::ExecutionResult;
use fluentbase_types::{ExitCode, STATE_DEPLOY, STATE_MAIN};
use rwasm::TrapCode;

#[cfg(feature = "std")]
use std::time::Instant;

/// Low-cardinality runtime mode label.
#[derive(Clone, Copy, Debug)]
pub enum RuntimeModeLabel {
    Contract,
    System,
}

impl RuntimeModeLabel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::System => "system",
        }
    }
}

/// Runtime entrypoint/state label.
pub fn state_label(state: u32) -> &'static str {
    match state {
        STATE_MAIN => "main",
        STATE_DEPLOY => "deploy",
        _ => "unknown",
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

/// Records an execution that failed before the runtime could be constructed.
pub fn record_initialization_error(mode: RuntimeModeLabel, state: &'static str, trap: TrapCode) {
    #[cfg(feature = "std")]
    {
        metrics::counter!(
            "fluentbase_runtime_initialization_errors_total",
            "mode" => mode.as_str(),
            "state" => state,
            "trap" => trap_label(trap),
        )
        .increment(1);
    }
}

/// Records one fresh runtime execution.
pub fn record_execution(
    mode: RuntimeModeLabel,
    state: &'static str,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    let outcome = result_outcome(result);
    #[cfg(feature = "std")]
    {
        record_common("execution", mode, state, outcome, timer, result);
        metrics::counter!(
            "fluentbase_runtime_executions_total",
            "mode" => mode.as_str(),
            "state" => state,
            "outcome" => outcome,
        )
        .increment(1);
        if outcome == "interrupted" {
            metrics::counter!(
                "fluentbase_runtime_interruptions_total",
                "mode" => mode.as_str(),
                "state" => state,
            )
            .increment(1);
        }
    }
}

/// Records one resume attempt.
pub fn record_resume(
    mode: RuntimeModeLabel,
    state: &'static str,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    let outcome = result_outcome(result);
    #[cfg(feature = "std")]
    {
        record_common("resume", mode, state, outcome, timer, result);
        metrics::counter!(
            "fluentbase_runtime_resumes_total",
            "mode" => mode.as_str(),
            "state" => state,
            "outcome" => outcome,
        )
        .increment(1);
        if outcome == "interrupted" {
            metrics::counter!(
                "fluentbase_runtime_interruptions_total",
                "mode" => mode.as_str(),
                "state" => state,
            )
            .increment(1);
        }
    }
}

/// Records that a suspended runtime was explicitly dropped.
pub fn record_forget_runtime(mode: RuntimeModeLabel, state: &'static str) {
    #[cfg(feature = "std")]
    metrics::counter!(
        "fluentbase_runtime_forgotten_total",
        "mode" => mode.as_str(),
        "state" => state,
    )
    .increment(1);
}

/// Publishes the current number of suspended runtimes held by the executor.
pub fn set_recoverable_runtimes(count: usize) {
    #[cfg(feature = "std")]
    metrics::gauge!("fluentbase_runtime_recoverable_runtimes").set(count as f64);
}

#[cfg(feature = "std")]
fn record_common(
    operation: &'static str,
    mode: RuntimeModeLabel,
    state: &'static str,
    outcome: &'static str,
    timer: &RuntimeTimer,
    result: &ExecutionResult,
) {
    let labels = [
        ("operation", operation),
        ("mode", mode.as_str()),
        ("state", state),
        ("outcome", outcome),
    ];
    metrics::histogram!("fluentbase_runtime_execution_seconds", &labels)
        .record(timer.elapsed_seconds());
    metrics::histogram!("fluentbase_runtime_fuel_consumed", &labels)
        .record(result.fuel_consumed as f64);
    metrics::histogram!("fluentbase_runtime_fuel_refunded", &labels)
        .record(result.fuel_refunded.max(0) as f64);
    metrics::histogram!("fluentbase_runtime_output_bytes", &labels)
        .record(result.output.len() as f64);
    metrics::histogram!("fluentbase_runtime_return_data_bytes", &labels)
        .record(result.return_data.len() as f64);
}

fn result_outcome(result: &ExecutionResult) -> &'static str {
    if result.exit_code > 0 {
        "interrupted"
    } else if result.exit_code == ExitCode::Ok.into_i32() {
        "success"
    } else if result.exit_code == ExitCode::OutOfFuel.into_i32() {
        "out_of_fuel"
    } else if result.exit_code == ExitCode::UnexpectedFatalExecutionFailure.into_i32() {
        "fatal"
    } else {
        "reverted"
    }
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
