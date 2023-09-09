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

const REM_SHIFT: usize = 3usize;
const REM_MASK: u64 = (1u64 << REM_SHIFT) - 1u64;
const I64_REM_SHIFT: usize = 60usize;
const I32_REM_SHIFT: usize = 28usize;

#[derive(Clone, Debug)]
pub(crate) struct WasmRelGadget<F> {
    // Neg version of arguments is used to reconstruct it from limbs than `is_neg` makes sense.
    lhs: AdviceColumn,
    is_neg_lhs: AdviceColumn,
    neg_lhs: AdviceColumn,
    rhs: AdviceColumn,
    is_neg_rhs: AdviceColumn,
    neg_rhs: AdviceColumn,

    // This limbs comes from absolute value.
    // So logic is to compare `is_neg` bits, and if it same than limbs can be used.
    lhs_limbs: [AdviceColumn; 8],
    rhs_limbs: [AdviceColumn; 8],
    neq_terms: [AdviceColumn; 8],
    out_terms: [AdviceColumn; 8],
    res: AdviceColumn,

    op_is_32bit: FixedColumn,
    op_is_eq: FixedColumn,
    op_is_ne: FixedColumn,
    op_is_lt: FixedColumn,
    op_is_gt: FixedColumn,
    op_is_le: FixedColumn,
    op_is_ge: FixedColumn,
    op_is_sign: FixedColumn,

    pd: PhantomData<F>,
}

// Idea it to make comparison for each limb, but only one result is correct.
// To filter this correct result, `ClzFilter` is used.
// Logic is skip equal limbs until we found difference.
impl<F: Field> ExecutionGadget<F> for WasmRelGadget<F> {
    const NAME: &'static str = "WASM_REL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_REL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs = cb.query_cell();
        let is_neg_lhs = cb.query_cell();
        let neg_lhs = cb.query_cell();
        let rhs = cb.query_cell();
        let is_neg_rhs = cb.query_cell();
        let neg_rhs = cb.query_cell();
        let res = cb.query_cell();

        let op_is_32bit = cb.query_fixed();
        let op_is_eq = cb.query_fixed();
        let op_is_ne = cb.query_fixed();
        let op_is_lt = cb.query_fixed();
        let op_is_gt = cb.query_fixed();
        let op_is_le = cb.query_fixed();
        let op_is_ge = cb.query_fixed();
        let op_is_sign = cb.query_fixed();

        let rhs_limbs = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];

        let lhs_limbs = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];

        let neq_terms = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];

        let out_terms = [
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
            cb.query_cell(),
        ];

        let op_is_64bit = || 1.expr() - op_is_32bit.expr();

        let is_pos_lhs = || 1.expr() - is_neg_lhs.expr();
        let is_pos_rhs = || 1.expr() - is_neg_rhs.expr();

        // Is must be three ones (without any zero), to be on same negative side, and 1 0 0 for
        // positive.
        let sign_and_all_neg = || op_is_sign.expr() * is_neg_lhs.expr() * is_neg_rhs.expr();
        let sign_and_all_pos = || op_is_sign.expr() * is_pos_lhs() * is_pos_rhs();

        // This logic is exclusive to previous one.
        let positive = || 1.expr() - op_is_sign.expr();

        // Means that fixed lookup table is used for real limbs, and this limbs has same sign.
        let enable = || sign_and_all_neg() + sign_and_all_pos() + positive();

        // Means that fixed lookup table for limbs is disabled, and we can just use bits of
        // negativity to make result. Means bits instead of real limbs, fake limbs created
        // from `is_pos_lhs` and `is_pos_rhs`.
        let disabled = || 1.expr() - enable();

        // Code like 0.expr() * op_is_nq.expr() is just skipped, because this is zero and exclusive
        // to other ops.
        let code = || {
            1.expr() * op_is_eq.expr()
                + 2.expr() * op_is_gt.expr()
                + 3.expr() * op_is_ge.expr()
                + 4.expr() * op_is_lt.expr()
                + 5.expr() * op_is_le.expr()
        };

        // To be correct comparison, if all on negative side, than limbs is inverted (negated as 255
        // - x).
        let mayinv = |limb: &AdviceColumn| {
            (positive() + sign_and_all_pos()) * limb.expr()
                + sign_and_all_neg() * (255.expr() - limb.expr())
        };

        for idx in 0..8 {
            cb.fixed_lookup(
                FixedTableTag::OpRel,
                [
                    mayinv(&lhs_limbs[idx]),
                    mayinv(&rhs_limbs[idx]) + 256.expr() * code(),
                    out_terms[idx].expr(),
                ]
                .map(|x| x * enable()),
            );
            cb.fixed_lookup(
                FixedTableTag::OpRel,
                [
                    lhs_limbs[idx].expr(),
                    rhs_limbs[idx].expr(),
                    neq_terms[idx].expr(),
                ]
                .map(|x| x * enable()),
            );
        }

        let mut neq_bits = neq_terms[0].expr();
        for neq_i in 1..8 {
            neq_bits = neq_bits + (1 << neq_i).expr() * neq_terms[neq_i].expr();
        }
        let mut out_bits = out_terms[0].expr();
        for out_i in 1..8 {
            out_bits = out_bits + (1 << out_i).expr() * out_terms[out_i].expr();
        }
        cb.fixed_lookup(
            FixedTableTag::ClzFilter,
            [neq_bits, out_bits, res.expr()].map(|x| x * enable()),
        );

        // Now constraints for `disabled` case, artifact fake limb is created from bits, sign is not
        // same. Positive versions is used because in this case positive is bigger than
        // negative.
        let fake_lhs_limb = || is_pos_lhs(); // Just simple aliases.
        let fake_rhs_limb = || is_pos_rhs();

        // Like previous lookup constraint, but for only first limb and term.
        cb.fixed_lookup(
            FixedTableTag::OpRel,
            [
                fake_lhs_limb(),
                fake_rhs_limb() + 256.expr() * code(),
                out_terms[0].expr(),
            ]
            .map(|x| x * disabled()),
        );
        // Neq term must be zero because fake limbs is different in disabled case.
        // If it is zero than `ClzFilter` will get smallest out_term as result of operation, so it
        // is not needed. This result of operation is comparsion operator of fake limbs
        // (different signs of lhs and rhs). Result is just in first out_term.
        cb.require_zeros(
            "op_rel: in case of disabled result is first term",
            [neq_terms[0].expr(), out_terms[0].expr() - res.expr()]
                .map(|x| x * disabled())
                .into(),
        );

        cb.require_zeros("op_rel: arguments from limbs", {
            let abs_lhs = || {
                let mut lhs_expr = lhs_limbs[0].expr();
                for i in 1..8 {
                    lhs_expr = lhs_expr + lhs_limbs[i].expr() * (1_u64 << i * 8).expr();
                }
                lhs_expr
            };
            let abs_rhs = || {
                let mut rhs_expr = rhs_limbs[0].expr();
                for i in 1..8 {
                    rhs_expr = rhs_expr + rhs_limbs[i].expr() * (1_u64 << i * 8).expr();
                }
                rhs_expr
            };
            vec![
                (abs_lhs() - lhs.expr()) * is_pos_lhs(),
                (abs_lhs() - neg_lhs.expr()) * is_neg_lhs.expr(),
                (abs_rhs() - rhs.expr()) * is_pos_rhs(),
                (abs_rhs() - neg_rhs.expr()) * is_neg_rhs.expr(),
            ]
        });

        // Sometimes modular zero is larger than u64, but it can fit into `Cell`, just adding one as
        // expr.
        let modular_zero32 = || 1.expr() + 0xffffffff_u64.expr();
        let modular_zero64 = || 1.expr() + 0xffffffff_ffffffff_u64.expr();

        cb.require_zeros(
            "op_rel: neg version is correct",
            vec![
                (neg_lhs.expr() + lhs.expr() - modular_zero32())
                    * is_neg_lhs.expr()
                    * op_is_32bit.expr(),
                (neg_rhs.expr() + rhs.expr() - modular_zero32())
                    * is_neg_rhs.expr()
                    * op_is_32bit.expr(),
                (neg_lhs.expr() + lhs.expr() - modular_zero64())
                    * is_neg_lhs.expr()
                    * op_is_64bit(),
                (neg_rhs.expr() + rhs.expr() - modular_zero64())
                    * is_neg_rhs.expr()
                    * op_is_64bit(),
            ],
        );

        cb.require_zeros(
            "op_rel: if 32bit then limbs must be zero",
            vec![{
                let mut check = 0.expr();
                for i in 4..8 {
                    check = check + rhs_limbs[i].expr() + lhs_limbs[i].expr();
                }
                check * op_is_32bit.expr()
            }],
        );

        cb.stack_pop(rhs.expr());
        cb.stack_pop(lhs.expr());
        cb.stack_push(res.expr());

        Self {
            lhs,
            is_neg_lhs,
            neg_lhs,
            rhs,
            is_neg_rhs,
            neg_rhs,
            lhs_limbs,
            rhs_limbs,
            neq_terms,
            out_terms,
            res,

            op_is_32bit,
            op_is_eq,
            op_is_ne,
            op_is_lt,
            op_is_gt,
            op_is_le,
            op_is_ge,
            op_is_sign,

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

        let lhs = trace.curr_nth_stack_value(1)?;
        let rhs = trace.curr_nth_stack_value(0)?;
        let res = trace.next_nth_stack_value(0)?;

        macro_rules! assigns {
            ($([$a:ident$([$b:ident])?, $c:expr])*) => {{
                $(self.$a$([$b])?.assign(region, offset, $c);)*
            }}
        }

        macro_rules! assign {
            ($a:ident, $b:expr) => {
                assigns! {[$a, $b]}
            };
        }
        macro_rules! assign_bits {($($a:ident),*) => { assigns! { $([ $a, 1 ])* } } }

        assigns! {
            [rhs, rhs.to_bits()]
            [lhs, lhs.to_bits()]
            [res, res.to_bits()]
        }

        let (is_neg_lhs, is_neg_rhs, abs_lhs, abs_rhs) = match opcode {
            Instruction::I32GtS
            | Instruction::I32GeS
            | Instruction::I32LtS
            | Instruction::I32LeS => {
                let is_neg_lhs = (lhs.as_u32() > i32::MAX as u32) as u64;
                let is_neg_rhs = (rhs.as_u32() > i32::MAX as u32) as u64;
                let abs_lhs = (lhs.as_u32() as i32).abs() as u64;
                let abs_rhs = (rhs.as_u32() as i32).abs() as u64;
                (is_neg_lhs, is_neg_rhs, abs_lhs, abs_rhs)
            }
            Instruction::I64GtS
            | Instruction::I64GeS
            | Instruction::I64LtS
            | Instruction::I64LeS => {
                let is_neg_lhs = (lhs.as_u64() > i64::MAX as u64) as u64;
                let is_neg_rhs = (rhs.as_u64() > i64::MAX as u64) as u64;
                let abs_lhs = (lhs.as_u64() as i64).abs() as u64;
                let abs_rhs = (rhs.as_u64() as i64).abs() as u64;
                (is_neg_lhs, is_neg_rhs, abs_lhs, abs_rhs)
            }
            _ => (0, 0, lhs.as_u64(), rhs.as_u64()),
        };

        assigns! {
            [is_neg_rhs, F::from(is_neg_rhs)]
            [is_neg_lhs, F::from(is_neg_lhs)]
        }

        if is_neg_rhs > 0 {
            assign!(neg_rhs, F::from(abs_rhs));
        }
        if is_neg_lhs > 0 {
            assign!(neg_lhs, F::from(abs_lhs));
        }

        // In case of signed or unsigned, then if this equal then real limbs is used.
        let enable = is_neg_rhs == is_neg_lhs;

        for idx in 0..8 {
            // This is additive inversion to make comparsion correct for case if all args on
            // negative side.
            let lhs_limb = (abs_lhs >> (8 * idx)) & 0xff;
            let rhs_limb = (abs_rhs >> (8 * idx)) & 0xff;
            let mut mi_lhs_limb = if is_neg_lhs > 0 {
                255_u64 - lhs_limb
            } else {
                lhs_limb
            };
            let mut mi_rhs_limb = if is_neg_rhs > 0 {
                255_u64 - rhs_limb
            } else {
                rhs_limb
            };
            if !enable {
                mi_lhs_limb = 1_u64 - is_neg_lhs;
                mi_rhs_limb = 1_u64 - is_neg_rhs;
            }
            let neq_out = mi_lhs_limb != mi_rhs_limb;
            let out = match opcode {
                Instruction::I32Ne | Instruction::I64Ne => neq_out,
                Instruction::I32Eq | Instruction::I64Eq => mi_lhs_limb == mi_rhs_limb,
                Instruction::I32GtU
                | Instruction::I64GtU
                | Instruction::I32GtS
                | Instruction::I64GtS => mi_lhs_limb > mi_rhs_limb,
                Instruction::I32GeU
                | Instruction::I64GeU
                | Instruction::I32GeS
                | Instruction::I64GeS => mi_lhs_limb >= mi_rhs_limb,
                Instruction::I32LtU
                | Instruction::I64LtU
                | Instruction::I32LtS
                | Instruction::I64LtS => mi_lhs_limb < mi_rhs_limb,
                Instruction::I32LeU
                | Instruction::I64LeU
                | Instruction::I32LeS
                | Instruction::I64LeS => mi_lhs_limb <= mi_rhs_limb,
                _ => false,
            };
            assigns! {
                [lhs_limbs[idx], F::from(lhs_limb)]
                [rhs_limbs[idx], F::from(rhs_limb)]
                [out_terms[idx], F::from(out)]
            }
            if enable {
                assigns! { [neq_terms[idx], F::from(neq_out)] }
                println!("DEBUG {idx} {lhs_limb} {rhs_limb} {neq_out} {out}");
            } else if idx == 0 {
                assigns! { [neq_terms[idx], F::from(0)] }
                println!("DEBUG {idx} {out}");
            }
        }

        let is_32 = match opcode {
            Instruction::I32GtU
            | Instruction::I32GeU
            | Instruction::I32LtU
            | Instruction::I32LeU
            | Instruction::I32Eq
            | Instruction::I32Ne
            | Instruction::I32GtS
            | Instruction::I32GeS
            | Instruction::I32LtS
            | Instruction::I32LeS => true,
            _ => false,
        };
        self.op_is_32bit.assign(region, offset, is_32 as u64);

        match opcode {
            Instruction::I32GtU | Instruction::I64GtU => assign_bits! { op_is_gt },
            Instruction::I32GeU | Instruction::I64GeU => assign_bits! { op_is_ge },
            Instruction::I32LtU | Instruction::I64LtU => assign_bits! { op_is_lt },
            Instruction::I32LeU | Instruction::I64LeU => assign_bits! { op_is_le },
            Instruction::I32Eq | Instruction::I64Eq => assign_bits! { op_is_eq },
            Instruction::I32Ne | Instruction::I64Ne => assign_bits! { op_is_ne },
            Instruction::I32GtS | Instruction::I64GtS => assign_bits! { op_is_gt, op_is_sign },
            Instruction::I32GeS | Instruction::I64GeS => assign_bits! { op_is_ge, op_is_sign },
            Instruction::I32LtS | Instruction::I64LtS => assign_bits! { op_is_lt, op_is_sign },
            Instruction::I32LeS | Instruction::I64LeS => assign_bits! { op_is_le, op_is_sign },
            _ => (),
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::{instruction_set, rwasm::InstructionSet};

    fn run_test(bytecode: InstructionSet) {
        test_ok(bytecode)
    }

    // Idea here is to run only lower triangle of pair matrix, and do four operations at once (four
    // tests inside). If any argument is not exist than test is skipped, also it is skippend out
    // of triangle.
    macro_rules! try_test_by_number {
        ([$Const:ident] [$op:ident] [$n:expr, $m:expr]) => {
            let run = || {
                let i = $n % $m;
                let j = $n / $m;
                if i >= j {
                    return Ok(());
                }
                let a = try_get_arg($n % $m)?;
                let b = try_get_arg($n / $m)?;
                run_test(instruction_set! {
                    $Const[a] $Const[a] $op Drop
                    $Const[b] $Const[b] $op Drop
                    $Const[a] $Const[b] $op Drop
                    $Const[b] $Const[a] $op Drop
                });
                Ok(())
            };
            let _: Result<(), ()> = run();
        };
    }

    macro_rules! tests_from_data {
        ([$( [$Const:ident [$($op:ident),*] [$($t:tt)*]] )*]) => {
            #[allow(non_snake_case)]
            mod generated_tests {
                use super::*;
                $(mod $Const {
                    use super::*;
                    fn try_get_arg(idx: usize) -> Result<i64, ()> {
                      vec![$($t)*].get(idx).ok_or(()).map(|x| *x)
                    }
                    $(mod $op {
                      use super::*;
                      use seq_macro::seq;
                      seq!(N in 0..100 {
                        #[test] fn test_~N() { try_test_by_number! { [$Const] [$op] [N, 10] } }
                      });
                    })*
                })*
            }
        }
    }

    // Example command to run test: cargo test generated_tests::I32Const::I32GtU::test_10
    // Encoding of test number is decimal pair by ten, ones and tens, a + b * 10
    // For example test_10 means lhs_index is 1 and rhs_index is 0
    // If `test_41` is used then do four comparisons with "-1" and "1" etc, see
    // `try_test_by_number`.
    tests_from_data! {
      [
        [I32Const
          [I32GtU, I32GeU, I32LtU, I32LeU, I32Eq, I32Ne, I32GtS, I32GeS, I32LtS, I32LeS]
          [0, 1, 2, -1, -2, 0x80000000]
        ]
        [I64Const
          [I64GtU, I64GeU, I64LtU, I64LeU, I64Eq, I64Ne, I64GtS, I64GeS, I64LtS, I64LeS]
          [0, 1, 2, -1, -2, -0x100000001, -0x100000002, 0x100000001, 0x100000002]
        ]
      ]
    }
}
