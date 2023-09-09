use crate::{
    constraint_builder::{AdviceColumn, FixedColumn, ToExpr},
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
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpUnaryGadget<F> {
    operand: AdviceColumn,
    result: AdviceColumn,
    is_ctz: FixedColumn,
    is_clz: FixedColumn,
    is_popcnt: FixedColumn,
    is_64bits: FixedColumn,
    arg_limbs: [AdviceColumn; 8],
    terms: [AdviceColumn; 4],
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpUnaryGadget<F> {
    const NAME: &'static str = "WASM_UNARY";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_UNARY;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let operand = cb.query_cell();
        let result = cb.query_cell();

        let is_ctz = cb.query_fixed();
        let is_clz = cb.query_fixed();
        let is_popcnt = cb.query_fixed();
        let is_64bits = cb.query_fixed();
        let is_32bits = || 1.expr() - is_64bits.expr();

        let arg_limbs = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];
        let terms = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];

        cb.stack_pop(operand.expr());
        cb.stack_push(result.expr());

        for i in 0..4 {
            let even = || arg_limbs[i * 2].expr();
            let odd = || arg_limbs[i * 2 + 1].expr();
            cb.fixed_lookup(
                FixedTableTag::Ctz,
                [
                    even() * is_ctz.expr(),
                    odd() * is_ctz.expr(),
                    terms[i].expr() * is_ctz.expr() + 16.expr() * (1.expr() - is_ctz.expr()),
                ],
            );
            cb.fixed_lookup(
                FixedTableTag::Clz,
                [
                    even() * is_clz.expr(),
                    odd() * is_clz.expr(),
                    terms[i].expr() * is_clz.expr() + 16.expr() * (1.expr() - is_clz.expr()),
                ],
            );
            cb.fixed_lookup(
                FixedTableTag::PopCnt,
                [
                    even() * is_popcnt.expr(),
                    odd() * is_popcnt.expr(),
                    terms[i].expr() * is_popcnt.expr(),
                ],
            );
        }

        cb.fixed_lookup(
            FixedTableTag::CzOut,
            [
                (terms[0].expr() + terms[1].expr() * 17.expr()) * is_ctz.expr(),
                (terms[2].expr() + terms[3].expr() * 17.expr()) * is_64bits.expr() * is_ctz.expr(),
                result.expr() * is_ctz.expr(),
            ],
        );
        cb.fixed_lookup(
            FixedTableTag::CzOut,
            [
                ((terms[3].expr() + terms[2].expr() * 17.expr()) * is_64bits.expr()
                    + (terms[1].expr() + terms[0].expr() * 17.expr()) * is_32bits())
                    * is_clz.expr(),
                (terms[1].expr() + terms[0].expr() * 17.expr()) * is_64bits.expr() * is_clz.expr(),
                result.expr() * is_clz.expr(),
            ],
        );

        cb.require_zero(
            "op_unary: selector",
            is_ctz.expr() + is_clz.expr() + is_popcnt.expr() - 1.expr(),
        );

        cb.require_zeros(
            "op_unary: argument from limbs",
            vec![{
                let mut out = arg_limbs[0].expr();
                for i in 1..8 {
                    out = out + arg_limbs[i].expr() * (1_u64 << i * 8).expr();
                }
                out - operand.expr()
            }],
        );

        cb.require_zeros(
            "op_unary: popcnt",
            vec![
                (terms[0].expr() + terms[1].expr() + terms[2].expr() + terms[3].expr()
                    - result.expr())
                    * is_popcnt.expr(),
            ],
        );

        Self {
            operand,
            result,
            is_ctz,
            is_clz,
            is_popcnt,
            is_64bits,
            arg_limbs,
            terms,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let opcode = trace.instr();

        let operand = trace.curr_nth_stack_value(0)?;
        let result = trace.next_nth_stack_value(0)?;

        self.operand.assign(region, offset, operand.to_bits());
        self.result.assign(region, offset, result.to_bits());

        let (selector, bits, _max) = match opcode {
            Instruction::I32Ctz => (&self.is_ctz, 32, 1u128 << 32),
            Instruction::I64Ctz => (&self.is_ctz, 64, 1u128 << 64),
            Instruction::I32Clz => (&self.is_clz, 32, 1u128 << 32),
            Instruction::I64Clz => (&self.is_clz, 64, 1u128 << 64),
            Instruction::I32Popcnt => (&self.is_popcnt, 32, 1u128 << 32),
            Instruction::I64Popcnt => (&self.is_popcnt, 64, 1u128 << 64),
            _ => unreachable!("not supported opcode for unary operation: {:?}", opcode),
        };
        selector.assign(region, offset, F::one());
        self.is_64bits
            .assign(region, offset, F::from((bits == 64) as u64));

        for idx in 0..4 {
            let pair = (operand.to_bits() >> (idx * 16)) & 0xffff;
            let even = pair & 0xff;
            let odd = pair >> 8;
            self.arg_limbs[idx * 2].assign(region, offset, F::from(even));
            self.arg_limbs[idx * 2 + 1].assign(region, offset, F::from(odd));
            match opcode {
                Instruction::I32Ctz | Instruction::I64Ctz => {
                    self.terms[idx].assign(
                        region,
                        offset,
                        F::from(bitintr::Tzcnt::tzcnt(pair as u16) as u64),
                    );
                }
                Instruction::I32Clz | Instruction::I64Clz => {
                    self.terms[idx].assign(
                        region,
                        offset,
                        F::from(bitintr::Lzcnt::lzcnt(pair as u16) as u64),
                    );
                }
                Instruction::I32Popcnt | Instruction::I64Popcnt => {
                    self.terms[idx].assign(region, offset, F::from(bitintr::Popcnt::popcnt(pair)));
                }
                _ => unreachable!("not supported opcode for unary operation: {:?}", opcode),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_ctz() {
        test_ok(instruction_set! {
            I32Const[0x00100000]
            I32Ctz
            Drop
            I32Const[0x00000001]
            I32Ctz
            Drop
            I32Const[0x80000000u32 as i32]
            I32Ctz
            Drop
            I32Const[0x00000000]
            I32Ctz
            Drop
            I64Const[0x0010000000000000u64 as i64]
            I64Ctz
            Drop
            I64Const[0x0000000000000001]
            I64Ctz
            Drop
            I64Const[0x8000000000000000u64 as i64]
            I64Ctz
            Drop
            I64Const[0x0000000000000000]
            I64Ctz
            Drop
        });
    }

    #[test]
    fn test_clz() {
        test_ok(instruction_set! {
            I32Const[0x00000001]
            I32Clz
            Drop
            I32Const[0x80000000u32 as i32]
            I32Clz
            Drop
            I32Const[0x00000000]
            I32Clz
            Drop
            I32Const[0xffffffffu32 as i32]
            I32Clz
            Drop
            I64Const[0x0000000000000001]
            I64Clz
            Drop
            I64Const[0x8000000000000000u64 as i64]
            I64Clz
            Drop
            I64Const[0x0000000000000000]
            I64Clz
            Drop
            I64Const[0xffffffffffffffffu64 as i64]
            I64Clz
            Drop
        });
    }

    #[test]
    fn test_popcnt32() {
        test_ok(instruction_set! {
            I32Const[0x00000000]
            I32Popcnt
            Drop
            I32Const[0xffffffffu32 as i32]
            I32Popcnt
            Drop
        });
    }

    #[test]
    fn test_popcnt64() {
        test_ok(instruction_set! {
            I64Const[0x0000000000000000]
            I64Popcnt
            Drop
            I64Const[0xffffffffffffffffu64 as i64]
            I64Popcnt
            Drop
        });
    }
}
