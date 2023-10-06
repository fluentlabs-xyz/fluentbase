use crate::{
    constraint_builder::ToExpr,
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::{marker::PhantomData, ops::Add};

#[derive(Clone, Debug)]
pub(crate) struct OpExtendGadget<F> {
    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpExtendGadget<F> {
    const NAME: &'static str = "WASM_BITWISE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_BITWISE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        Self {
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use log::debug;
    use rand::{thread_rng, Rng};

    const MAX: i32 = 10000;

    fn gen_params<const N: usize>() -> [i32; N] {
        let params = [0; N].map(|i| thread_rng().gen_range(0..MAX * 2) - MAX);
        debug!("params {:?}", params);
        params
    }

    #[test]
    fn test() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32And

            Drop
        });
    }
}
