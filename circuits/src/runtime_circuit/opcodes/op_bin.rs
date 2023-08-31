use crate::{
    constraint_builder::{AdviceColumn, Query, ToExpr},
    runtime_circuit::{
        execution_state::ExecutionState,
        opcodes::{ExecutionGadget, OpConstraintBuilder},
    },
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{
    circuit::{Region, Value},
    plonk::Error,
};

#[derive(Clone, Debug)]
pub(crate) struct WasmBinGadget<F: Field> {
    lhs: AdviceColumn,
    lhs_neg: AdviceColumn,
    rhs: AdviceColumn,
    rhs_neg: AdviceColumn,
    res: AdviceColumn,
    res_neg: AdviceColumn,
    is_add: AdviceColumn,
    is_sub: AdviceColumn,
    is_mul: AdviceColumn,
    is_div_u: AdviceColumn,
    is_rem_u: AdviceColumn,
    is_div_s: AdviceColumn,
    is_rem_s: AdviceColumn,
    div_rem_s_is_lhs_pos: AdviceColumn,
    div_rem_s_is_rhs_pos: AdviceColumn,
    aux1: AdviceColumn,
    aux1_neg: AdviceColumn,
    aux2: AdviceColumn,
    aux2_neg: AdviceColumn,
    aux3: AdviceColumn,
    aux3_neg: AdviceColumn,
    is_64bits: AdviceColumn,
}

impl<F: Field> ExecutionGadget<F> for WasmBinGadget<F> {
    const NAME: &'static str = "WASM_BIN";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_BIN;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs = cb.query_cell();
        let lhs_neg = cb.query_cell();
        let rhs = cb.query_cell();
        let rhs_neg = cb.query_cell();
        let res = cb.query_cell();
        let res_neg = cb.query_cell();

        let is_add = cb.query_cell();
        let is_sub = cb.query_cell();
        let is_mul = cb.query_cell();
        let is_div_u = cb.query_cell();
        let is_rem_u = cb.query_cell();
        let is_div_s = cb.query_cell();
        let is_rem_s = cb.query_cell();

        let div_rem_s_is_lhs_pos = cb.query_cell();
        let div_rem_s_is_rhs_pos = cb.query_cell();

        let aux1 = cb.query_cell();
        let aux1_neg = cb.query_cell();
        let aux2 = cb.query_cell();
        let aux2_neg = cb.query_cell();
        let aux3 = cb.query_cell();
        let aux3_neg = cb.query_cell();

        // let lhs_flag = cb.alloc_bit_value();
        // let rhs_flag = cb.alloc_bit_value();

        // let lhs_flag_helper = cb.alloc_common_range_value();
        // let lhs_flag_helper_diff = cb.alloc_common_range_value();
        // let rhs_flag_helper = cb.alloc_common_range_value();
        // let rhs_flag_helper_diff = cb.alloc_common_range_value();
        // let d_flag_helper_diff = cb.alloc_common_range_value();

        let is_64bits = cb.alloc_bit_value();

        cb.stack_pop(rhs.expr());
        cb.stack_pop(lhs.expr());
        cb.stack_push(res.expr());

        // TODO: Analyze the security of such an addition. In theory, if all the `is` variables have
        // already been proven as the only possible one or zero, then there is no problem.
        // If `alloc_bit_value` does the job. If not, then fraud is possible.
        cb.require_equal(
            "binop: selector",
            is_add.expr()
                + is_sub.expr()
                + is_mul.expr()
                + is_div_u.expr()
                + is_rem_u.expr()
                + is_div_s.expr()
                + is_rem_s.expr(),
            1.expr(),
        );

        let modulus = Query::Constant(F::from(1u64 << 32usize))
            + Query::Constant(F::from((u32::MAX as u64) << 32usize)) * is_64bits.expr();

        cb.require_zero(
            "binop: add constraint",
            (lhs.expr() + rhs.expr() - res.expr() - aux1.expr() * modulus.clone()) * is_add.expr(),
        );

        cb.require_zero(
            "binop: sub constraint",
            (rhs.expr() + res.expr() - lhs.expr() - aux1.expr() * modulus.clone()) * is_sub.expr(),
        );

        cb.require_zero(
            "binop: mul constraint",
            (lhs.expr() * rhs.expr() - aux1.expr() * modulus.clone() - res.expr()) * is_mul.expr(),
        );

        cb.require_zeros(
            "div_u/rem_u constraints",
            vec![
                (lhs.expr() - rhs.expr() * aux1.expr() - aux2.expr())
                    * (is_rem_u.expr() + is_div_u.expr()),
                (aux2.expr() + aux3.expr() + 1.expr() - rhs.expr())
                    * (is_rem_u.expr() + is_div_u.expr()),
                (res.expr() - aux1.expr()) * is_div_u.expr(),
                (res.expr() - aux2.expr()) * is_rem_u.expr(),
            ],
        );

        let pp_case = |xc| xc * div_rem_s_is_lhs_pos.expr() * div_rem_s_is_rhs_pos.expr();
        cb.require_zeros(
            "div_s/rem_s constraints pp case",
            vec![
                (lhs.expr() - rhs.expr() * aux1.expr() - aux2.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (aux2.expr() + aux3.expr() + 1.expr() - rhs.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (res.expr() - aux1.expr()) * is_div_s.expr(),
                (res.expr() - aux2.expr()) * is_rem_s.expr(),
            ]
            .into_iter()
            .map(pp_case)
            .collect(),
        );

        // Conversion is used, if we know that number is non-zero and negative.
        let conv_32 = |x| 0xffffffff_u64.expr() - x + 1.expr();
        let conv_64 = |x| 0xffffffff_ffffffff_u64.expr() - x + 1.expr();
        let is_64bits_f = |xc| xc * is_64bits.expr();
        let is_32bits_f = |xc| xc * (1.expr() - is_64bits.expr());

        // For this constraints to work correctly, check that if negative is same than it must be
        // zero. To make this check you can see than constraint is like duplicated.
        // So both direct and negative version must be zero at the same time, if constrait
        // substration is failing.
        macro_rules! make_cnr_constraint {
            ($name:expr, $conv:expr, $f:expr) => {
                cb.require_zeros(
                    $name,
                    vec![
                        (lhs.expr() - $conv(lhs_neg.expr())) * lhs.expr(),
                        (lhs.expr() - $conv(lhs_neg.expr())) * lhs_neg.expr(),
                        (rhs.expr() - $conv(rhs_neg.expr())) * rhs.expr(),
                        (rhs.expr() - $conv(rhs_neg.expr())) * rhs_neg.expr(),
                        (res.expr() - $conv(res_neg.expr())) * res.expr(),
                        (res.expr() - $conv(res_neg.expr())) * res_neg.expr(),
                        (aux1.expr() - $conv(aux1_neg.expr())) * aux1.expr(),
                        (aux1.expr() - $conv(aux1_neg.expr())) * aux1_neg.expr(),
                        (aux2.expr() - $conv(aux2_neg.expr())) * aux2.expr(),
                        (aux2.expr() - $conv(aux2_neg.expr())) * aux2_neg.expr(),
                        (aux3.expr() - $conv(aux3_neg.expr())) * aux3.expr(),
                        (aux3.expr() - $conv(aux3_neg.expr())) * aux3_neg.expr(),
                    ]
                    .into_iter()
                    .map($f)
                    .collect(),
                );
            };
        }
        make_cnr_constraint!("check negatives, rules for 64 bits", conv_64, is_64bits_f);
        make_cnr_constraint!("check negatives, rules for 32 bits", conv_32, is_32bits_f);

        let pn_case =
            |xc| xc * div_rem_s_is_lhs_pos.expr() * (1.expr() - div_rem_s_is_rhs_pos.expr());
        cb.require_zeros(
            "div_s/rem_s constraints pn case",
            vec![
                (lhs.expr() - rhs_neg.expr() * aux1_neg.expr() - aux2.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (aux3_neg.expr() - aux2.expr() - 1.expr() - rhs_neg.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (res.expr() - aux1.expr()) * is_div_s.expr(),
                (res.expr() - aux2.expr()) * is_rem_s.expr(),
            ]
            .into_iter()
            .map(pn_case)
            .collect(),
        );

        let np_case =
            |xc| xc * (1.expr() - div_rem_s_is_lhs_pos.expr()) * div_rem_s_is_rhs_pos.expr();
        cb.require_zeros(
            "div_s/rem_s constraints np case",
            vec![
                (lhs_neg.expr() - rhs.expr() * aux1_neg.expr() - aux2_neg.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (aux3_neg.expr() + aux2_neg.expr() - 1.expr() - rhs_neg.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (res.expr() - aux1.expr()) * is_div_s.expr(),
                (res.expr() - aux2.expr()) * is_rem_s.expr(),
            ]
            .into_iter()
            .map(np_case)
            .collect(),
        );

        let nn_case = |xc| {
            xc * (1.expr() - div_rem_s_is_lhs_pos.expr()) * (1.expr() - div_rem_s_is_rhs_pos.expr())
        };
        cb.require_zeros(
            "div_s/rem_s constraints nn case",
            vec![
                (lhs_neg.expr() - rhs_neg.expr() * aux1.expr() - aux2_neg.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (aux3_neg.expr() + aux2_neg.expr() - 1.expr() - rhs_neg.expr())
                    * (is_rem_s.expr() + is_div_s.expr()),
                (res.expr() - aux1.expr()) * is_div_s.expr(),
                (res.expr() - aux2.expr()) * is_rem_s.expr(),
            ]
            .into_iter()
            .map(nn_case)
            .collect(),
        );

        Self {
            lhs,
            lhs_neg,
            rhs,
            rhs_neg,
            res,
            res_neg,
            is_add,
            is_sub,
            is_mul,
            is_div_u,
            is_rem_u,
            is_div_s,
            is_rem_s,
            div_rem_s_is_lhs_pos,
            div_rem_s_is_rhs_pos,
            aux1,
            aux1_neg,
            aux2,
            aux2_neg,
            aux3,
            aux3_neg,
            is_64bits,
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let rhs = trace.curr_nth_stack_value(0)?;
        let lhs = trace.curr_nth_stack_value(1)?;
        let res = trace.next_nth_stack_value(0)?;

        self.lhs
            .assign(region, offset, Value::known(lhs.to_scalar().unwrap()))?;
        self.rhs
            .assign(region, offset, Value::known(rhs.to_scalar().unwrap()))?;
        self.res
            .assign(region, offset, Value::known(res.to_scalar().unwrap()))?;

        let selector = match trace.instr() {
            Instruction::I32Add | Instruction::I64Add => &self.is_add,
            Instruction::I32Sub | Instruction::I64Sub => &self.is_sub,
            Instruction::I32Mul | Instruction::I64Mul => &self.is_mul,
            Instruction::I32DivS | Instruction::I64DivS => &self.is_div_s,
            Instruction::I32DivU | Instruction::I64DivU => &self.is_div_u,
            Instruction::I32RemU | Instruction::I64RemU => &self.is_rem_u,
            Instruction::I32RemS | Instruction::I64RemS => &self.is_rem_s,
            _ => unreachable!("not supported opcode: {:?}", trace.instr()),
        };
        selector.assign(region, offset, Value::known(F::one()))?;

        let aux1;
        let mut aux2 = 0u64;
        let mut aux3 = 0u64;

        let mut div_rem_s_is_lhs_pos = 0u64;
        let mut div_rem_s_is_rhs_pos = 0u64;

        match trace.instr() {
            Instruction::I32Add => {
                let (_, overflow) = (lhs.as_u32()).overflowing_add(rhs.as_u32());
                aux1 = overflow as u64
            }
            Instruction::I64Add => {
                let (_, overflow) = lhs.overflowing_add(rhs);
                aux1 = overflow as u64
            }
            Instruction::I32Sub => {
                let (_, overflow) = (lhs.as_u32()).overflowing_sub(rhs.as_u32());
                aux1 = overflow as u64
            }
            Instruction::I64Sub => {
                let (_, overflow) = lhs.overflowing_sub(rhs);
                aux1 = overflow as u64
            }
            Instruction::I32Mul => {
                let (res2, overflow) = (lhs.as_u64()).overflowing_mul(rhs.as_u64());
                debug_assert!(!overflow, "overflow here is not possible");
                aux1 = res2 >> 32;
            }
            Instruction::I64Mul => {
                let (res2, overflow) = (lhs.as_u64() as u128).overflowing_mul(rhs.as_u64() as u128);
                debug_assert!(!overflow, "overflow here is not possible");
                aux1 = (res2 >> 64) as u64;
            }
            Instruction::I32DivU | Instruction::I32RemU => {
                aux1 = (lhs.as_u32() / rhs.as_u32()) as u64;
                aux2 = (lhs.as_u32() % rhs.as_u32()) as u64;
                aux3 = (rhs.as_u32() - lhs.as_u32() % rhs.as_u32() - 1) as u64;
            }
            Instruction::I64DivU | Instruction::I64RemU => {
                aux1 = (lhs.as_u64() / rhs.as_u64()) as u64;
                aux2 = (lhs.as_u64() % rhs.as_u64()) as u64;
                aux3 = (rhs.as_u64() - lhs.as_u64() % rhs.as_u64() - 1) as u64;
            }
            Instruction::I32DivS | Instruction::I32RemS => {
                // TODO: check and correct to fix possible problems with conversion.
                aux1 = ((lhs.as_u32() as i32 / rhs.as_u32() as i32) as u32) as u64;
                aux2 = ((lhs.as_u32() as i32 % rhs.as_u32() as i32) as u32) as u64;
                aux3 = ((rhs.as_u32() as i32 - lhs.as_u32() as i32 % rhs.as_u32() as i32 - 1)
                    as u32) as u64;
                div_rem_s_is_lhs_pos = (lhs.as_u32() <= i32::MAX as u32) as u64;
                div_rem_s_is_rhs_pos = (rhs.as_u32() <= i32::MAX as u32) as u64;
            }
            Instruction::I64DivS | Instruction::I64RemS => {
                // TODO: check and correct to fix possible problems with conversion.
                aux1 = (lhs.as_u64() as i64 / rhs.as_u64() as i64) as u64;
                aux2 = (lhs.as_u64() as i64 % rhs.as_u64() as i64) as u64;
                aux3 = (rhs.as_u64() as i64 - lhs.as_u64() as i64 % rhs.as_u64() as i64 - 1) as u64;
                div_rem_s_is_lhs_pos = (lhs.as_u64() <= i64::MAX as u64) as u64;
                div_rem_s_is_rhs_pos = (rhs.as_u64() <= i64::MAX as u64) as u64;
            }
            _ => unreachable!("not supported opcode: {:?}", trace.instr()),
        };
        self.aux1
            .assign(region, offset, Value::known(F::from(aux1)))?;
        self.aux2
            .assign(region, offset, Value::known(F::from(aux2)))?;
        self.aux3
            .assign(region, offset, Value::known(F::from(aux3)))?;
        self.div_rem_s_is_lhs_pos.assign(
            region,
            offset,
            Value::known(F::from(div_rem_s_is_lhs_pos)),
        )?;
        self.div_rem_s_is_rhs_pos.assign(
            region,
            offset,
            Value::known(F::from(div_rem_s_is_rhs_pos)),
        )?;

        let is_64bit = matches!(
            opcode,
            Instruction::I64Add
                | Instruction::I64Sub
                | Instruction::I64Mul
                | Instruction::I64DivS
                | Instruction::I64DivU
                | Instruction::I64RemS
                | Instruction::I64RemU
        );
        self.is_64bits
            .assign(region, offset, Value::known(F::from(is_64bit as u64)))?;

        let mut rhs_neg = 0u64;
        let mut lhs_neg = 0u64;
        let mut res_neg = 0u64;
        let mut aux1_neg = 0u64;
        let mut aux2_neg = 0u64;
        let mut aux3_neg = 0u64;

        if is_64bit {
            rhs_neg = (rhs.0[0] as i64).neg() as u64;
            lhs_neg = (lhs.0[0] as i64).neg() as u64;
            res_neg = (res.0[0] as i64).neg() as u64;
            aux1_neg = (aux1 as i64).neg() as u64;
            aux2_neg = (aux2 as i64).neg() as u64;
            aux3_neg = (aux3 as i64).neg() as u64;
        } else {
            rhs_neg = ((rhs.0[0] as i32).neg() as u32) as u64;
            lhs_neg = ((lhs.0[0] as i32).neg() as u32) as u64;
            res_neg = ((res.0[0] as i32).neg() as u32) as u64;
            aux1_neg = ((aux1 as i32).neg() as u32) as u64;
            aux2_neg = ((aux2 as i32).neg() as u32) as u64;
            aux3_neg = ((aux3 as i32).neg() as u32) as u64;
        }

        self.rhs_neg
            .assign(region, offset, Value::known(F::from(rhs_neg)))?;
        self.lhs_neg
            .assign(region, offset, Value::known(F::from(lhs_neg)))?;
        self.res_neg
            .assign(region, offset, Value::known(F::from(res_neg)))?;
        self.aux1_neg
            .assign(region, offset, Value::known(F::from(aux1_neg)))?;
        self.aux2_neg
            .assign(region, offset, Value::known(F::from(aux2_neg)))?;
        self.aux3_neg
            .assign(region, offset, Value::known(F::from(aux3_neg)))?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::test_util::CircuitTestBuilder;
    use eth_types::{bytecode, Bytecode};
    use mock::TestContext;

    fn run_test(bytecode: Bytecode) {
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(bytecode).unwrap(),
        )
        .run()
    }

    #[test]
    fn test_i32_add() {
        run_test(bytecode! {
            I32Const[1]
            I32Const[1]
            I32Add
            Drop
        });
    }

    #[test]
    fn test_i32_add_overflow() {
        run_test(bytecode! {
            I32Const[1]
            I32Const[4294967295]
            I32Add
            Drop
        });
    }

    #[test]
    fn test_i64_add() {
        run_test(bytecode! {
            I64Const[1]
            I64Const[1]
            I64Add
            Drop
        });
    }

    #[test]
    fn test_i64_add_overflow() {
        run_test(bytecode! {
            I64Const[1]
            I64Const[18446744073709551615]
            I64Add
            Drop
        });
    }

    #[test]
    fn test_i32_mul() {
        run_test(bytecode! {
            I32Const[3]
            I32Const[4]
            I32Mul
            Drop
        });
    }

    #[test]
    fn test_i32_mul_overflow() {
        run_test(bytecode! {
            I32Const[4294967295]
            I32Const[4294967295]
            I32Mul
            Drop
        });
    }

    #[test]
    fn test_i32_div_u() {
        run_test(bytecode! {
            I32Const[4]
            I32Const[3]
            I32DivU
            Drop
        });
        run_test(bytecode! {
            I32Const[0x80000000]
            I32Const[1]
            I32DivU
            Drop
        });
    }

    #[test]
    fn test_i64_mul() {
        run_test(bytecode! {
            I64Const[3]
            I64Const[4]
            I64Mul
            Drop
        });
    }

    #[test]
    fn test_i64_mul_overflow() {
        run_test(bytecode! {
            I64Const[18446744073709551615]
            I64Const[18446744073709551615]
            I64Mul
            Drop
        });
    }

    #[test]
    fn test_i32_64_rem() {
        run_test(bytecode! {
            I64Const[4]
            I64Const[3]
            I64RemU
            Drop
            I64Const[4]
            I64Const[4]
            I64RemU
            Drop
        });
    }

    macro_rules! div_rem_s_pat {
        ($A:ident, $B:ident) => {
            run_test(bytecode! {
                $A[-4] $A[-3] $B Drop
                $A[-4] $A[ 3] $B Drop
                $A[ 4] $A[-3] $B Drop
                $A[ 4] $A[-4] $B Drop
                $A[-3] $A[-3] $B Drop
            });
        };
    }

    macro_rules! make_div_rem_s_tests {
      ($([$name:ident, $A:ident, $B:ident])*) => {$(
        #[test]
        fn $name() {
          div_rem_s_pat!($A, $B);
        }
      )*}
    }

    make_div_rem_s_tests! {
        [test_64_rem_s, I64Const, I64RemS]
        [test_64_div_s, I64Const, I64RemS]
        [test_32_rem_s, I32Const, I32RemS]
        [test_32_div_s, I32Const, I32RemS]
    }

    // `s_pp` means signed where lhs is positive and rhs is positive.
    #[test]
    fn test_i32_64_rem_s_pp() {
        run_test(bytecode! {
            I64Const[4]
            I64Const[3]
            I64RemS
            Drop
            I64Const[4]
            I64Const[4]
            I64RemS
            Drop
        });
    }

    // `s_pp` means signed where lhs is positive and rhs is positive.
    #[test]
    fn test_i32_64_div_s_pp() {
        run_test(bytecode! {
            I64Const[4]
            I64Const[3]
            I64DivS
            Drop
            I64Const[4]
            I64Const[4]
            I64DivS
            Drop
        });
    }

    // `s_pp` means signed where lhs is positive and rhs is positive.
    #[test]
    fn test_i32_32_rem_s_pp() {
        run_test(bytecode! {
            I32Const[4]
            I32Const[3]
            I32RemS
            Drop
            I32Const[4]
            I32Const[4]
            I32RemS
            Drop
        });
    }

    // `s_pp` means signed where lhs is positive and rhs is positive.
    #[test]
    fn test_i32_32_div_s_pp() {
        run_test(bytecode! {
            I32Const[4]
            I32Const[3]
            I32DivS
            Drop
            I32Const[4]
            I32Const[4]
            I32DivS
            Drop
        });
    }

    #[test]
    fn test_different_cases() {
        run_test(bytecode! {
            I32Const[100]
            I32Const[20]
            I32Add
            I32Const[3]
            I32Add
            I32Const[123]
            I32Sub
            Drop
        });
    }
}
