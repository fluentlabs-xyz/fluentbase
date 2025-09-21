use crate::{instruction::exec::InterruptionHolder, ExecutionResult};
use fluentbase_types::{Bytes, CALL_DEPTH_ROOT, STATE_MAIN};

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    /// A fuel limit for this call. If it's set to `None` then fuel is disabled.
    pub fuel_limit: u64,
    /// Do we disable fuel for this call?
    /// With this flag enabled,
    /// we ignore fuel checks inside VMs and the only way to manage fuel is by using builtins:
    /// `charge_fuel` and `charge_fuel_manually`
    pub disable_fuel: bool,
    /// An execution state.
    /// Usually we use values like `STATE_MAIN (0)`
    /// and `STATE_DEPLOY (1)` to identify application execution state: deployment or execution.
    /// Theoretically, other states can be used, but we don't use it right now.
    pub state: u32,
    /// A call depth for this execution phrase.
    /// Once it reached `MAX_CALL_DEPTH (1024)`, then execution halts.
    /// By checking call depth to zero, we determine so-called root layer.
    pub call_depth: u32,
    /// An input for the application.
    /// Just an input.
    /// It already contains all required context params inside.
    pub input: Bytes,
    /// Execution result details, like exit code or output data.
    pub execution_result: ExecutionResult,
    /// An information about interrupted call.
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
    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    pub fn with_input<I: Into<Bytes>>(mut self, input_data: I) -> Self {
        self.input = input_data.into();
        self
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    pub fn with_call_depth(mut self, depth: u32) -> Self {
        self.call_depth = depth;
        self
    }

    pub fn with_disabled_fuel(mut self) -> Self {
        self.disable_fuel = true;
        self
    }

    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }

    pub fn is_fuel_disabled(&self) -> bool {
        self.disable_fuel
    }
}
