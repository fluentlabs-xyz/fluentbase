use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    fixed_table::FixedTableTag,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::{marker::PhantomData, ops::Add};

const LIMBS_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct OpShiftGadget<F> {
    // Instruction::I32Shl,
    // Instruction::I64Shl,
    // Instruction::I32ShrS,
    // Instruction::I32ShrU,
    // Instruction::I64ShrS,
    // Instruction::I64ShrU,
    is_i32shl: SelectorColumn,
    is_i64shl: SelectorColumn,
    is_i32shr_s: SelectorColumn,
    is_i32shr_u: SelectorColumn,
    is_i64shr_s: SelectorColumn,
    is_i64shr_u: SelectorColumn,

    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpShiftGadget<F> {
    const NAME: &'static str = "WASM_SHIFT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_SHIFT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32shl = cb.query_selector();
        let is_i64shl = cb.query_selector();
        let is_i32shr_s = cb.query_selector();
        let is_i32shr_u = cb.query_selector();
        let is_i64shr_s = cb.query_selector();
        let is_i64shr_u = cb.query_selector();

        Self {
            is_i32shl,
            is_i64shl,
            is_i32shr_s,
            is_i32shr_u,
            is_i64shr_s,
            is_i64shr_u,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let p = trace.curr_nth_stack_value(0)?.to_bits();
        let r = trace.next_nth_stack_value(0)?.to_bits();

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use log::debug;
    use rand::{thread_rng, Rng};

    fn gen_params<const N: usize, const MAX_POSITIVE_VAL: i64>() -> [i64; N] {
        let params = [0; N]
            .map(|i| thread_rng().gen_range(0..=MAX_POSITIVE_VAL * 2 + 1) - MAX_POSITIVE_VAL - 1);
        debug!("params {:?}", params);
        params
    }

    #[test]
    fn test_i32shl() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        let [p1, p2] = [1, 2];
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32Shl

            Drop
        });
    }

    #[test]
    fn test_i64shl() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I64Shl

            Drop
        });
    }

    #[test]
    fn test_i32shr_s() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32ShrS

            Drop
        });
    }

    #[test]
    fn test_i32shr_u() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32ShrU

            Drop
        });
    }

    #[test]
    fn test_i64shr_s() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I64ShrS

            Drop
        });
    }

    #[test]
    fn test_i64shr_u() {
        let [p1, p2] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I64ShrU

            Drop
        });
    }
}
