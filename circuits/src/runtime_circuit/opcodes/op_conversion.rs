use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
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
pub(crate) struct OpConversionGadget<F> {
    value: AdviceColumn,
    value_limbs: [AdviceColumn; 8],
    res: AdviceColumn,
    is_value_pos: AdviceColumn,
    is_i32_wrap_i64: AdviceColumn,
    is_i64_extend_i32_u: AdviceColumn,
    is_i64_extend_i32_s: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpConversionGadget<F> {
    const NAME: &'static str = "WASM_CONVERSION";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CONVERSION;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();
        let value_limbs = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];
        let res = cb.query_cell();

        let is_value_pos = cb.query_cell();
        let is_i32_wrap_i64 = cb.query_cell();
        let is_i64_extend_i32_u = cb.query_cell();
        let is_i64_extend_i32_s = cb.query_cell();

        cb.stack_pop(value.expr());
        cb.stack_push(res.expr());

        // for i in 0..4 {
        //     cb.add_lookup(
        //         "op_conversion: Using Range256x2 fixed table",
        //         Lookup::Fixed {
        //             tag: FixedTableTag::Range256x2.expr(),
        //             values: [
        //                 value_limbs[i * 2].expr(),
        //                 value_limbs[i * 2 + 1].expr(),
        //                 0.expr(),
        //             ],
        //         },
        //     );
        // }

        cb.require_zeros(
            "op_conversion: pick one",
            vec![
                is_i32_wrap_i64.expr() + is_i64_extend_i32_u.expr() + is_i64_extend_i32_s.expr()
                    - 1.expr(),
            ],
        );

        cb.require_zeros(
            "op_conversion: argument from limbs",
            vec![{
                let mut out = value_limbs[0].expr();
                for i in 1..8 {
                    out = out + value_limbs[i].expr() * (1_u64 << i * 8).expr();
                }
                out - value.expr()
            }],
        );

        cb.require_zeros(
            "op_conversion: result from limbs in case of i32_wrap_i64",
            vec![{
                let mut out = value_limbs[0].expr();
                for i in 1..4 {
                    out = out + value_limbs[i].expr() * (1_u64 << i * 8).expr();
                }
                (out - res.expr()) * is_i32_wrap_i64.expr()
            }],
        );

        cb.require_zeros("op_conversion: result case of i64_extend_i32", {
            // Now we are working with i32 that can be signed or not, in both cases only first four
            // limbs is used. So limbs goes after this must be zero, this check is used
            // to make sure about it.
            let mut check = value_limbs[4].expr();
            for i in 5..8 {
                check = check + value_limbs[i].expr();
            }
            let cond = || is_i64_extend_i32_u.expr() + is_i64_extend_i32_s.expr();
            let pos_cond =
                || is_i64_extend_i32_u.expr() + is_i64_extend_i32_s.expr() * is_value_pos.expr();
            let neg_cond = || is_i64_extend_i32_s.expr() * (1.expr() - is_value_pos.expr());
            vec![
                check * cond(),
                (value.expr() - res.expr()) * pos_cond(),
                (value.expr() + 0xffffffff_00000000_u64.expr() - res.expr()) * neg_cond(),
            ]
        });

        Self {
            value,
            value_limbs,
            res,
            is_value_pos,
            is_i32_wrap_i64,
            is_i64_extend_i32_u,
            is_i64_extend_i32_s,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let opcode = trace.instr();

        let value = trace.curr_nth_stack_value(0)?;
        let res = trace.next_nth_stack_value(0)?;

        self.value.assign(region, offset, value.to_bits());
        self.res.assign(region, offset, res.to_bits());

        for idx in 0..8 {
            let limb = (value.to_bits() >> (idx * 8)) & 0xff;
            self.value_limbs[idx].assign(region, offset, F::from(limb));
        }

        match opcode {
            Instruction::I32WrapI64 => {
                self.is_i32_wrap_i64.assign(region, offset, true as u64);
            }
            Instruction::I64ExtendI32U => {
                self.is_i64_extend_i32_u.assign(region, offset, true as u64);
            }
            Instruction::I64ExtendI32S => {
                let is_value_pos = (value.as_u32() <= i32::MAX as u32) as u64;
                self.is_value_pos.assign(region, offset, is_value_pos);
                self.is_i64_extend_i32_s.assign(region, offset, true as u64);
            }
            _ => unreachable!("not supported opcode: {:?}", opcode),
        };

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_i32_wrap_i64() {
        test_ok(instruction_set! {
            I64Const[0]
            I32WrapI64
            Drop
            I64Const[0xffffffff00000000u64 as i64]
            I32WrapI64
            Drop
            I64Const[0xfffffffff0f0f0f0u64 as i64]
            I32WrapI64
            Drop
        });
    }

    #[test]
    fn test_i64_extend_u_i32() {
        test_ok(instruction_set! {
            I32Const[0]
            I64ExtendI32U
            Drop
            I32Const[0xffffffffu32 as i32]
            I64ExtendI32U
            Drop
            I32Const[0x0f0f0f0fu32 as i32]
            I64ExtendI32U
            Drop
        });
    }

    #[test]
    fn test_i64_extend_s_i32() {
        test_ok(instruction_set! {
            I32Const[0]
            I64ExtendI32S
            Drop
            I32Const[0x70ffffff]
            I64ExtendI32S
            Drop
            I32Const[-0x70ffffff]
            I64ExtendI32S
            Drop
        });
    }
}
