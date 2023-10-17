use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpConsumeFuel<F> {
    consumed_fuel: AdviceColumn,
    value: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpConsumeFuel<F> {
    const NAME: &'static str = "WASM_CONSUME_FUEL";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CONSUME_FUEL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let consumed_fuel = cb.query_cell();
        let value = cb.query_cell();
        cb.context_lookup(
            RwTableContextTag::ConsumedFuel,
            1.expr(),
            consumed_fuel.current() + value.current(),
            Some(consumed_fuel.current()),
        );
        // TODO: "add OutOfFuel error check"
        Self {
            consumed_fuel,
            value,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &ExecStep,
    ) -> Result<(), GadgetError> {
        let consumed_fuel = step.curr().consumed_fuel;
        self.consumed_fuel.assign(region, offset, consumed_fuel);
        let value = step.instr().aux_value().unwrap_or_default();
        self.value.assign(region, offset, value.as_u64());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_consume_fuel() {
        test_ok(instruction_set! {
            ConsumeFuel(0)
            ConsumeFuel(1)
            ConsumeFuel(2)
            ConsumeFuel(3)
        });
    }
}
