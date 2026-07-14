use crate::{executor::ExecutionResult, syscall_handler::InterruptionHolder};
use fluentbase_types::{Bytes, ExitCode, CALL_DEPTH_ROOT, STATE_MAIN};

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
    pub fn take_resumable_context_serialized(&mut self) -> Result<Option<Vec<u8>>, ExitCode> {
        // Take resumable context from execution context
        let Some(resumable_context) = self.resumable_context.take() else {
            return Ok(None);
        };
        if resumable_context.is_root {
            return Err(ExitCode::UnexpectedFatalExecutionFailure);
        }
        // serialize the delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        let result = resumable_context.params.encode();
        Ok(Some(result))
    }

    /// Clears the accumulated output buffer.
    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_types::{SyscallInvocationParams, B256};

    fn resumable_context(is_root: bool) -> InterruptionHolder {
        InterruptionHolder {
            params: SyscallInvocationParams {
                code_hash: B256::default(),
                input: 4..8,
                fuel_limit: 13,
                state: 2,
                fuel16_ptr: 16,
            },
            is_root,
        }
    }

    #[test]
    fn root_resumable_context_returns_fatal_error_without_panicking() {
        let mut ctx = RuntimeContext {
            resumable_context: Some(resumable_context(true)),
            ..Default::default()
        };

        let err = ctx.take_resumable_context_serialized().unwrap_err();

        assert_eq!(err, ExitCode::UnexpectedFatalExecutionFailure);
        assert!(ctx.resumable_context.is_none());
    }

    #[test]
    fn child_resumable_context_still_serializes() {
        let params = resumable_context(false).params;
        let mut ctx = RuntimeContext {
            resumable_context: Some(InterruptionHolder {
                params: params.clone(),
                is_root: false,
            }),
            ..Default::default()
        };

        let encoded = ctx
            .take_resumable_context_serialized()
            .unwrap()
            .expect("child interruption should serialize");

        assert_eq!(SyscallInvocationParams::decode(&encoded), Some(params));
        assert!(ctx.resumable_context.is_none());
    }
}
