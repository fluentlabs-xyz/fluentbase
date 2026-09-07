use crate::RuntimeContext;
use core::mem::take;
use rwasm::{StoreTr, StrategyExecutor, TrapCode};
use std::sync::atomic::{AtomicUsize, Ordering};

static GUEST_PROFILE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

pub(super) fn capture_guest_coverage_profile(
    executor: &mut StrategyExecutor<RuntimeContext>,
    state: Option<u32>,
) -> Result<Option<Vec<u8>>, TrapCode> {
    let mut previous_context = RuntimeContext::default();
    core::mem::swap(executor.data_mut(), &mut previous_context);
    if let Some(state) = state {
        executor.data_mut().state = state;
    }

    // Coverage serialization is diagnostic work and must not change the fuel observed by the
    // caller. Give it an isolated budget, then restore the exact pre-capture remainder.
    let previous_fuel = executor.remaining_fuel();
    if previous_fuel.is_some() {
        executor.reset_fuel(u64::MAX);
    }

    let capture_result = executor.execute("__fluentbase_coverage_dump", &[], &mut []);
    let profile = take(&mut executor.data_mut().execution_result.output);

    if let Some(previous_fuel) = previous_fuel {
        executor.reset_fuel(previous_fuel);
    }

    // The diagnostic call must not alter the execution result returned to the caller.
    core::mem::swap(executor.data_mut(), &mut previous_context);

    match capture_result {
        Ok(()) if !profile.is_empty() => Ok(Some(profile)),
        Ok(()) | Err(TrapCode::UnknownExternalFunction) => Ok(None),
        Err(trap) => Err(trap),
    }
}

pub(super) fn write_guest_coverage_profile(
    executor: &mut StrategyExecutor<RuntimeContext>,
    state: Option<u32>,
) {
    let Some(output_dir) = std::env::var_os("FLUENTBASE_GUEST_PROFILE_DIR") else {
        return;
    };

    let profile = match capture_guest_coverage_profile(executor, state) {
        Ok(Some(profile)) => profile,
        Ok(None) => return,
        Err(error) => {
            eprintln!("failed to capture guest coverage: {error}");
            return;
        }
    };

    let output_dir = std::path::PathBuf::from(output_dir);
    if let Err(error) = std::fs::create_dir_all(&output_dir) {
        eprintln!("failed to create guest coverage directory: {error}");
        return;
    }

    let process_id = std::process::id();
    let sequence = GUEST_PROFILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let output_path = output_dir.join(format!("guest-{process_id}-{sequence}.profraw"));
    if let Err(error) = std::fs::write(&output_path, profile) {
        eprintln!(
            "failed to write guest coverage profile {}: {error}",
            output_path.display()
        );
    }
}
