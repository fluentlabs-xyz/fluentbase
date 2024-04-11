//! Custom print inspector, it has step level information of execution.
//! It is a great tool if some debugging is needed.

use crate::types::CallInputs;
use crate::types::CallOutcome;
use crate::types::CreateInputs;
use crate::types::CreateOutcome;
use crate::types::Interpreter;
use crate::{
    inspectors::GasInspector,
    primitives::{Address, U256},
    EvmContext, Inspector,
};
use fluentbase_types::IJournaledTrie;

/// Custom print [Inspector], it has step level information of execution.
///
/// It is a great tool if some debugging is needed.
#[derive(Clone, Debug, Default)]
pub struct CustomPrintTracer {
    gas_inspector: GasInspector,
}

impl<DB: IJournaledTrie> Inspector<DB> for CustomPrintTracer {
    fn initialize_interp(&mut self, interp: &mut Interpreter, context: &mut EvmContext<DB>) {
        self.gas_inspector.initialize_interp(interp, context);
    }

    // get opcode by calling `interp.contract.opcode(interp.program_counter())`.
    // all other information can be obtained from interp.
    fn step(&mut self, _interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        // let opcode = interp.current_opcode;
        // let opcode_str = opcode::OPCODE_JUMPMAP[opcode as usize];
        //
        // let gas_remaining = self.gas_inspector.gas_remaining();
        //
        // let memory_size = interp.shared_memory.len();
        //
        // println!(
        //     "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}",
        //     context.journaled_state.depth(),
        //     interp.program_counter(),
        //     gas_remaining,
        //     gas_remaining,
        //     opcode_str.unwrap_or("UNKNOWN"),
        //     opcode,
        //     interp.gas.refunded(),
        //     interp.gas.refunded(),
        //     interp.stack.data(),
        //     memory_size,
        // );
        //
        // self.gas_inspector.step(interp, context);
    }

    fn step_end(&mut self, interp: &mut Interpreter, context: &mut EvmContext<DB>) {
        self.gas_inspector.step_end(interp, context);
    }

    fn call_end(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        self.gas_inspector.call_end(context, inputs, outcome)
    }

    fn create_end(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        self.gas_inspector.create_end(context, inputs, outcome)
    }

    fn call(
        &mut self,
        _context: &mut EvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        println!(
            "SM CALL:   {:?}, context:{:?}, is_static:{:?}, transfer:{:?}, input_size:{:?}",
            inputs.contract,
            inputs.context,
            inputs.is_static,
            inputs.transfer,
            inputs.input.len(),
        );
        None
    }

    fn create(
        &mut self,
        _context: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        println!(
            "CREATE CALL: caller:{:?}, scheme:{:?}, value:{:?}, init_code:{:?}, gas:{:?}",
            inputs.caller, inputs.scheme, inputs.value, inputs.init_code, inputs.gas_limit
        );
        None
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        println!(
            "SELFDESTRUCT: contract: {:?}, refund target: {:?}, value {:?}",
            contract, target, value
        );
    }
}
