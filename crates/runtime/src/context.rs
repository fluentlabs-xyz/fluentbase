use crate::{executor::ExecutionResult, syscall_handler::InterruptionHolder};
use fluentbase_types::{Bytes, CALL_DEPTH_ROOT, STATE_MAIN};

/// Per-invocation execution context carried inside the VM store.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    /// Maximum fuel available to this invocation.
    pub fuel_limit: u64,
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

    /// Extract serialized resumable context
    pub fn take_resumable_context_serialized(&mut self) -> Option<Vec<u8>> {
        // Take resumable context from execution context
        let resumable_context = self.resumable_context.take()?;
        if resumable_context.is_root {
            unimplemented!("validate this logic, might not be ok in STF mode");
        }
        // serialize the delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        let result = resumable_context.params.encode();
        Some(result)
    }

    /// Clears the accumulated output buffer.
    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }
}
