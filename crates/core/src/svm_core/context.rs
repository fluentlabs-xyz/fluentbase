use fluentbase_sdk::SharedAPI;
use solana_rbpf::static_analysis::TraceLogEntry;
use solana_rbpf::vm::ContextObject;
use alloc::vec::Vec;

#[derive(Debug, Clone, Default)]
pub struct ExecContextObject<SDK: SharedAPI> {
    pub trace_log: Vec<TraceLogEntry>,
    pub remaining: u64,
    pub sdk: SDK,
}

impl<'a, SDK: SharedAPI> ContextObject for ExecContextObject<SDK> {
    fn trace(&mut self, state: [u64; 12]) {
        self.trace_log.push(state);
    }

    fn consume(&mut self, amount: u64) {
        self.remaining = self.remaining.saturating_sub(amount);
    }

    fn get_remaining(&self) -> u64 {
        self.remaining
    }
}

impl<SDK: SharedAPI> ExecContextObject<SDK> {
    /// Initialize with instruction meter
    pub fn new(sdk: SDK, remaining: u64) -> Self {
        Self {
            trace_log: Vec::new(),
            remaining,
            sdk,
        }
    }

    /// Compares an interpreter trace and a JIT trace.
    ///
    /// The log of the JIT can be longer because it only validates the instruction meter at branches.
    pub fn compare_trace_log(interpreter: &Self, jit: &Self) -> bool {
        let interpreter = interpreter.trace_log.as_slice();
        let mut jit = jit.trace_log.as_slice();
        if jit.len() > interpreter.len() {
            jit = &jit[0..interpreter.len()];
        }
        interpreter == jit
    }
}