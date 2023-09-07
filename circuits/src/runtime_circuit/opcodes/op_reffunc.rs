use crate::{
    bail_illegal_opcode,
    constraint_builder::AdviceColumn,
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

#[derive(Clone, Debug)]
pub(crate) struct OpRefFuncGadget<F: Field> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpRefFuncGadget<F> {
    const NAME: &'static str = "WASM_REFFUNC";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_REFFUNC;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();
        cb.stack_push(value.current());
        Self {
            value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::RefFunc(val) => val,
            _ => bail_illegal_opcode!(trace),
        };
        let value = trace.next_nth_stack_value(0)?;
        self.value.assign(region, offset, value.to_bits());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn reffunc_stack_top_offset() {
        test_ok(instruction_set! {
            RefFunc(0)
            Drop
        });
    }

    #[test]
    fn reffunc_stack_depth() {
        test_ok(instruction_set! {
            RefFunc(0)
            RefFunc(0)
            RefFunc(0)
            Drop
            Drop
            Drop
        });
    }
}
