use crate::{
    bail_illegal_opcode,
    constraint_builder::FixedColumn,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpCallGadget<F: Field> {
    is_host_call: FixedColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpCallGadget<F> {
    const NAME: &'static str = "WASM_CALL";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CALL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_host_call = cb.query_fixed();

        cb.if_rwasm_opcode(
            is_host_call.current(),
            Instruction::Call(Default::default()),
            |_cb| {},
        );

        Self {
            is_host_call,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::Return(_) => {}
            Instruction::Call(_) => {
                self.is_host_call.assign(region, offset, 1u64);
            }
            _ => bail_illegal_opcode!(trace),
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_runtime::SysFuncIdx;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_exit() {
        test_ok(instruction_set! {
            I32Const(7)
            Call(SysFuncIdx::IMPORT_SYS_HALT)
        });
    }
}
