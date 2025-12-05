use fluentbase_sdk::{Address, Log, U256};
use revm::{
    context::ContextTr,
    interpreter::{
        interpreter_types::{Jumps, MemoryTr},
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter,
    },
    state, Inspector,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterpreterState {
    pub(crate) pc: usize,
    pub(crate) stack_len: usize,
    pub(crate) memory_size: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StepRecord {
    pub(crate) before: InterpreterState,
    pub(crate) after: Option<InterpreterState>,
    pub(crate) opcode_name: String,
    pub(crate) opcode: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InspectorEvent {
    Step(StepRecord),
    Call {
        inputs: CallInputs,
        outcome: Option<CallOutcome>,
    },
    Create {
        inputs: CreateInputs,
        outcome: Option<CreateOutcome>,
    },
    Log(Log),
    #[allow(dead_code)]
    Selfdestruct {
        address: Address,
        beneficiary: Address,
        value: U256,
    },
}

pub(crate) struct TraceInspector {
    pub(crate) events: Vec<InspectorEvent>,
    pub(crate) step_count: usize,
    pub(crate) call_depth: usize,
}

impl TraceInspector {
    pub(crate) fn new() -> Self {
        Self {
            events: vec![],
            step_count: 0,
            call_depth: 0,
        }
    }

    fn capture_interpreter_state(interp: &Interpreter) -> InterpreterState {
        InterpreterState {
            pc: interp.bytecode.pc(),
            stack_len: interp.stack.len(),
            memory_size: interp.memory.size(),
        }
    }
}

impl<CTX: ContextTr> Inspector<CTX> for TraceInspector {
    fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        self.step_count += 1;

        let state = Self::capture_interpreter_state(interp);
        let opcode = interp.bytecode.opcode();
        let opcode_name = if let Some(op) = state::bytecode::opcode::OpCode::new(opcode) {
            format!("{op}")
        } else {
            format!("Unknown(0x{opcode:02x})")
        };

        self.events.push(InspectorEvent::Step(StepRecord {
            before: state,
            after: None,
            opcode_name,
            opcode,
        }));
    }

    fn step_end(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        let state = Self::capture_interpreter_state(interp);

        if let Some(InspectorEvent::Step(record)) = self.events.last_mut() {
            record.after = Some(state);
        }
    }

    fn log(&mut self, _ctx: &mut CTX, log: Log) {
        self.events.push(InspectorEvent::Log(log));
    }

    fn call(&mut self, _ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.call_depth += 1;
        self.events.push(InspectorEvent::Call {
            inputs: inputs.clone(),
            outcome: None,
        });
        None
    }

    fn call_end(&mut self, _ctx: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.call_depth -= 1;
        if let Some(InspectorEvent::Call {
            outcome: ref mut out,
            ..
        }) = self
            .events
            .iter_mut()
            .rev()
            .find(|e| matches!(e, InspectorEvent::Call { outcome: None, .. }))
        {
            *out = Some(outcome.clone());
        }
    }

    fn create(&mut self, _ctx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.events.push(InspectorEvent::Create {
            inputs: inputs.clone(),
            outcome: None,
        });
        None
    }

    fn create_end(&mut self, _ctx: &mut CTX, _inputs: &CreateInputs, outcome: &mut CreateOutcome) {
        if let Some(InspectorEvent::Create {
            outcome: ref mut out,
            ..
        }) = self
            .events
            .iter_mut()
            .rev()
            .find(|e| matches!(e, InspectorEvent::Create { outcome: None, .. }))
        {
            *out = Some(outcome.clone());
        }
    }

    fn selfdestruct(&mut self, _contract: Address, _beneficiary: Address, _value: U256) {
        // self.events.push(InspectorEvent::Selfdestruct {
        //     address: contract,
        //     beneficiary,
        //     value,
        // });
    }
}
