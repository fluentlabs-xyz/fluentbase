use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn, ToExpr},
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

const LIMBS_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct OpShiftGadget<F> {
    // Instruction::I32Shl
    // Instruction::I32ShrU
    // Instruction::I32ShrS
    // Instruction::I64Shl
    // Instruction::I64ShrS
    // Instruction::I64ShrU
    // Instruction::I32Rotr
    // Instruction::I32Rotl
    // Instruction::I64Rotl
    // Instruction::I64Rotr
    is_i32shl: SelectorColumn,
    is_i64shl: SelectorColumn,
    is_i32shr_s: SelectorColumn,
    is_i32shr_u: SelectorColumn,
    is_i64shr_s: SelectorColumn,
    is_i64shr_u: SelectorColumn,

    value: AdviceColumn,
    shift: AdviceColumn,
    res: AdviceColumn,

    shift32_low5bit_and_rest: [AdviceColumn; 2],
    shift64_low6bit_and_rest: [AdviceColumn; 2],

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

        let [value, shift, res] = cb.query_cells();
        let shift32_low5bit_and_rest = cb.query_cells();
        let shift64_low6bit_and_rest = cb.query_cells();

        cb.stack_pop(shift.current());
        cb.stack_pop(value.current());
        cb.stack_push(res.current());

        cb.require_equal(
            "shift=reconstruct(shift32_low5bit_and_rest)",
            shift.current(),
            shift32_low5bit_and_rest[0].current()
                + shift32_low5bit_and_rest[1].current() * Query::from(0b100000),
        );
        cb.require_equal(
            "shift=reconstruct(shift64_low6bit_and_rest)",
            shift.current(),
            shift64_low6bit_and_rest[0].current()
                + shift64_low6bit_and_rest[1].current() * Query::from(0b1000000),
        );

        Self {
            is_i32shl,
            is_i64shl,
            is_i32shr_s,
            is_i32shr_u,
            is_i64shr_s,
            is_i64shr_u,
            shift,
            value,
            res,
            shift32_low5bit_and_rest,
            shift64_low6bit_and_rest,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let shift = trace.curr_nth_stack_value(0)?.to_bits();
        let value = trace.curr_nth_stack_value(1)?.to_bits();
        let res = trace.next_nth_stack_value(0)?.to_bits();

        self.value.assign(region, offset, value);
        self.shift.assign(region, offset, shift);
        self.res.assign(region, offset, res);

        self.shift32_low5bit_and_rest[0].assign(region, offset, shift % 0b100000);
        self.shift32_low5bit_and_rest[1].assign(region, offset, shift / 0b100000);
        self.shift64_low6bit_and_rest[0].assign(region, offset, shift % 0b1000000);
        self.shift64_low6bit_and_rest[1].assign(region, offset, shift / 0b1000000);

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
        let [v, s] = gen_params::<2, 0b1111111>();
        let [v, s] = [2, 1];
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32Shl

            Drop
        });
    }

    #[test]
    fn test_i64shl() {
        let [v, s] = gen_params::<2, 0b1111111>();
        let [v, s] = [3, 64];
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64Shl

            Drop
        });
    }

    #[test]
    fn test_i32shr_s() {
        let [v, s] = gen_params::<2, 0b1111111>();
        let [v, s] = [-3, 32];
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32ShrS

            Drop
        });
    }

    #[test]
    fn test_i32shr_u() {
        let [v, s] = gen_params::<2, 0b1111111>();
        let [v, s] = [-3, 32];
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32ShrU

            Drop
        });
    }

    #[test]
    fn test_i64shr_s() {
        let [v, s] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64ShrS

            Drop
        });
    }

    #[test]
    fn test_i64shr_u() {
        let [v, s] = gen_params::<2, 0b1111111>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64ShrU

            Drop
        });
    }
}
