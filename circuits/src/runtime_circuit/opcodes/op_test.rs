use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    gadgets::binary_number::AsBits,
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
use std::{marker::PhantomData, ops::Neg};

#[derive(Clone, Debug)]
pub(crate) struct OpTestGadget<F> {
    is_i64: AdviceColumn,
    value_inv: AdviceColumn,
    value: AdviceColumn,
    is_eqz: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTestGadget<F> {
    const NAME: &'static str = "WASM_TEST";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TEST;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();
        let is_eqz = cb.query_cell();
        let is_i64 = cb.query_cell();
        let value_inv = cb.query_cell();

        cb.stack_pop(value.expr());
        cb.stack_push(is_eqz.expr());

        cb.require_zero(
            "is_eqz==0=>value!=0 || is_eqz=1=>value=0",
            is_eqz.current() * value.current(),
        );

        cb.require_zero(
            "two",
            value.current() * value_inv.current() - 1.expr() + is_eqz.current(),
        );

        Self {
            is_i64,
            value_inv,
            value,
            is_eqz,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let value = trace.curr_nth_stack_value(0)?;
        let res = trace.curr_nth_stack_value(0)?;

        let is_i64 = matches!(trace.instr(), Instruction::I64Eqz);
        self.is_i64.assign(region, offset, is_i64);

        let value_neg = if is_i64 {
            F::from(value.as_u64()).invert().unwrap_or(F::zero())
        } else {
            F::from(value.as_u32() as u64).invert().unwrap_or(F::zero())
        };

        self.value.assign(region, offset, value.as_u64());
        self.value_inv.assign(region, offset, value_neg);
        self.is_eqz.assign(region, offset, res.as_u64());

        match trace.instr() {
            Instruction::I64Eqz => {
                let is_eqz = (value.as_u64() == 0) as u64;
                self.is_eqz.assign(region, offset, is_eqz);
            }
            Instruction::I32Eqz => {
                let is_eqz = (value.as_u32() == 0) as u64;
                self.is_eqz.assign(region, offset, is_eqz);
            }
            _ => unreachable!("not supported opcode: {:?}", trace.instr()),
        };

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_i32_eqz1() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Eqz
            Drop
            I32Const[1]
            I32Eqz
            Drop
        });
    }

    #[test]
    fn test_i32_eqz2() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Eqz
            Drop
            I32Const[-1]
            I32Eqz
            Drop
        });
    }

    #[test]
    fn test_i32_eqz3() {
        test_ok(instruction_set! {
            I32Const[12]
            I32Eqz
            Drop
            I32Const[-13]
            I32Eqz
            Drop
        });
    }

    #[test]
    fn test_i64_eqz1() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Eqz
            Drop
            I64Const[1]
            I64Eqz
            Drop
        });
    }

    #[test]
    fn test_i64_eqz2() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Eqz
            Drop
            I64Const[-1]
            I64Eqz
            Drop
        });
    }

    #[test]
    fn test_i64_eqz3() {
        test_ok(instruction_set! {
            I64Const[12]
            I64Eqz
            Drop
            I64Const[-13]
            I64Eqz
            Drop
        });
    }
}
