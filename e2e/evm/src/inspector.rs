use crate::runner::TestError;
use fluentbase_sdk::{Address, Log, U256};
use revm::{
    bytecode::opcode,
    context::ContextTr,
    interpreter::{
        gas::MemoryGas,
        interpreter_types::{Jumps, MemoryTr},
        return_error, return_ok, return_revert, CallInputs, CallOutcome, CreateInputs,
        CreateOutcome, Interpreter,
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

#[allow(dead_code)]
pub fn check_evm_trace(
    inspector1: &mut TraceInspector,
    inspector2: &mut TraceInspector,
) -> Result<(), TestError> {
    let mut it1 = inspector1.events.iter_mut().filter(|e| match e {
        InspectorEvent::Step(step) => match step.opcode {
            opcode::CALL
            | opcode::STATICCALL
            | opcode::CALLCODE
            | opcode::DELEGATECALL
            | opcode::CREATE
            | opcode::CREATE2
            | opcode::STOP
            | opcode::RETURN
            | opcode::REVERT => true,
            _ => false,
        },
        _ => true,
    });
    let mut it2 = inspector2.events.iter_mut();
    let mut e1 = it1.next().unwrap();
    let mut e2 = it2.next().unwrap();
    loop {
        let (e1_next, e2_next) = match (&mut e1, &mut e2) {
            (
                InspectorEvent::Call {
                    inputs: _inputs1,
                    outcome: ref mut outcome1,
                },
                InspectorEvent::Call {
                    inputs: _inputs2,
                    outcome: ref mut outcome2,
                },
            ) => {
                println!("Call == Call");
                // assert_eq!(inputs1, inputs2);
                *outcome1.as_mut().unwrap().result.gas.memory_mut() = MemoryGas::new();
                *outcome2.as_mut().unwrap().result.gas.memory_mut() = MemoryGas::new();
                match (
                    &outcome1.as_ref().unwrap().result.result,
                    &outcome2.as_ref().unwrap().result.result,
                ) {
                    (return_ok!(), return_ok!()) => {}
                    (return_revert!(), return_revert!()) => {}
                    (return_error!(), return_error!()) => {}
                    (_, _) => assert_eq!(outcome1, outcome2),
                }
                assert_eq!(
                    outcome1.as_ref().unwrap().gas(),
                    outcome2.as_ref().unwrap().gas()
                );
                assert_eq!(
                    outcome1.as_ref().unwrap().output(),
                    outcome2.as_ref().unwrap().output()
                );
                (it1.next(), it2.next())
            }
            (
                InspectorEvent::Create {
                    inputs: _inputs1,
                    outcome: ref mut outcome1,
                },
                InspectorEvent::Create {
                    inputs: _inputs2,
                    outcome: ref mut outcome2,
                },
            ) => {
                println!("Create == Create");
                // assert_eq!(inputs1, inputs2);
                *outcome1.as_mut().unwrap().result.gas.memory_mut() = MemoryGas::new();
                *outcome2.as_mut().unwrap().result.gas.memory_mut() = MemoryGas::new();
                match (
                    &outcome1.as_ref().unwrap().result.result,
                    &outcome2.as_ref().unwrap().result.result,
                ) {
                    (return_ok!(), return_ok!()) => {}
                    (return_revert!(), return_revert!()) => {}
                    (return_error!(), return_error!()) => {}
                    (_, _) => assert_eq!(outcome1, outcome2),
                }
                assert_eq!(
                    outcome1.as_ref().unwrap().gas(),
                    outcome2.as_ref().unwrap().gas()
                );
                // assert_eq!(
                //     outcome1.as_ref().unwrap().output(),
                //     outcome2.as_ref().unwrap().output()
                // );
                assert_eq!(
                    outcome1.as_ref().unwrap().address,
                    outcome2.as_ref().unwrap().address,
                );
                (it1.next(), it2.next())
            }
            (InspectorEvent::Selfdestruct { .. }, _) => {
                // don't check selfdestruct events
                e1 = it1.next().unwrap();
                continue;
            }
            (InspectorEvent::Log(_), InspectorEvent::Log(_)) => {
                println!("Log == Log");
                assert_eq!(e1, e2);
                (it1.next(), it2.next())
            }
            (InspectorEvent::Step(step1), InspectorEvent::Step(step2)) => {
                println!(
                    "Opcode({}) == Opcode({})",
                    step1.opcode_name, step2.opcode_name
                );
                (it1.next(), it2.next())
            }
            (_, _) => {
                eprintln!("\n{:?} == {:?}", e1, e2);
                unreachable!()
            }
        };
        match (e1_next, e2_next) {
            (Some(e1_next), Some(e2_next)) => {
                e1 = e1_next;
                e2 = e2_next;
            }
            (None, None) => {
                break;
            }
            (Some(extra), None) => {
                eprintln!("{:?} == {:?}", e1, e2);
                eprintln!("Extra (EVM): {:?}", extra);
                unreachable!("oh, we have different number of events")
            }
            (None, Some(extra)) => {
                eprintln!("{:?} == {:?}", e1, e2);
                eprintln!("Extra (FLUENT): {:?}", extra);
                unreachable!("oh, we have different number of events")
            }
        }
    }
    Ok(())
}
