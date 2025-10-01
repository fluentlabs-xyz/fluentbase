use crate::{executor::ExecutionResult, syscall_handler::InterruptionHolder};
use fluentbase_types::{Bytes, CALL_DEPTH_ROOT, STATE_MAIN};

/// Per-invocation execution context carried inside the VM store.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    /// Maximum fuel available to this invocation.
    pub fuel_limit: u64,
    /// If true, the engine does not enforce fuel; fuel is managed explicitly by builtins.
    pub disable_fuel: bool,
    /// Entry selector for the module (e.g., STATE_MAIN or STATE_DEPLOY).
    pub state: u32,
    /// Current call depth; root is zero.
    pub call_depth: u32,
    /// Calldata for the invocation.
    pub input: Bytes,
    /// Mutable execution artifacts collected during the run.
    pub execution_result: ExecutionResult,
    /// Deferred invocation metadata used to resume an interrupted call.
    pub resumable_context: Option<InterruptionHolder>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            fuel_limit: 0,
            disable_fuel: false,
            state: STATE_MAIN,
            call_depth: CALL_DEPTH_ROOT,
            input: Bytes::default(),
            execution_result: ExecutionResult::default(),
            resumable_context: None,
        }
    }
}

impl RuntimeContext {
    /// Sets the fuel limit for this context.
    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    /// Replaces the calldata for this invocation.
    pub fn with_input<I: Into<Bytes>>(mut self, input_data: I) -> Self {
        self.input = input_data.into();
        self
    }

    /// Updates the entry selector (state) for this invocation.
    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    /// Sets the call depth for this context.
    pub fn with_call_depth(mut self, depth: u32) -> Self {
        self.call_depth = depth;
        self
    }

    /// Marks the context to run with fuel metering disabled.
    pub fn with_disabled_fuel(mut self) -> Self {
        self.disable_fuel = true;
        self
    }

    /// Clears the accumulated output buffer.
    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }

    /// Returns true if fuel metering is disabled for this context.
    pub fn is_fuel_disabled(&self) -> bool {
        self.disable_fuel
    }
}
