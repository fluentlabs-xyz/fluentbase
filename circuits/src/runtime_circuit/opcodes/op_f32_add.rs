use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    gadgets::{lt::LtGadget, is_zero::IsZeroConfig},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
        gadgets::f32_exp::F32ExpConfig,
        gadgets::f32_mantissa::F32MantissaConfig,
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
    fixed_table::FixedTableTag,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;
use crate::constraint_builder::SelectorColumn;
use crate::constraint_builder::Query;
use std::ops::Not;

#[derive(Clone)]
pub(crate) struct OpF32AddGadget<F: Field> {
    lhs_exp_ge: SelectorColumn,
    out_exp_ge: SelectorColumn,
    lhs_sign: SelectorColumn,
    rhs_sign: SelectorColumn,
    out_sign: SelectorColumn,
    lhs_exp: F32ExpConfig<F>,
    rhs_exp: F32ExpConfig<F>,
    out_exp: F32ExpConfig<F>,
    lhs_mant: F32MantissaConfig<F>,
    rhs_mant: F32MantissaConfig<F>,
    out_mant: F32MantissaConfig<F>,
    pow2: AdviceColumn,
    // 24 bits to compare for addition, that is simpler than multiplication.
    lt_gadget: Option<LtGadget<F, 4>>,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32AddGadget<F> {
    const NAME: &'static str = "WASM_F32_ADD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_ADD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs_exp_ge = cb.query_selector();
        let out_exp_ge = cb.query_selector();

        let lhs_sign = cb.query_selector();
        let rhs_sign = cb.query_selector();
        let out_sign = cb.query_selector();

        let lhs_exp = cb.query_f32_exp();
        let rhs_exp = cb.query_f32_exp();
        let out_exp = cb.query_f32_exp();

        let lhs_mant = cb.query_f32_mantissa(lhs_exp.is_extra_bit_set().into());
        let rhs_mant = cb.query_f32_mantissa(rhs_exp.is_extra_bit_set().into());
        let out_mant = cb.query_f32_mantissa(out_exp.is_extra_bit_set().into());

        let pow2 = cb.query_cell();

        let lhs_sign_bit = || -> Query<F> { lhs_sign.current().into() };

        cb.range_check8(lhs_exp_ge.current().select(lhs_exp.current() - rhs_exp.current(), 0.expr()));
        //cb.range_check8(out_exp_ge.current().select(out_exp.current() - rhs_exp.current(), 0.expr()));

        cb.stack_pop(
            rhs_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + rhs_exp.current() * 0x800000.expr()
                + rhs_mant.raw_part(rhs_exp.is_extra_bit_set().into())
        );

        cb.stack_pop(
            lhs_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + lhs_exp.current() * 0x800000.expr()
                + lhs_mant.raw_part(lhs_exp.is_extra_bit_set().into())
        );

        cb.stack_push(
            out_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + out_exp.current() * 0x800000.expr()
                + out_mant.raw_part(out_exp.is_extra_bit_set().into())
        );

        // Halo field is used to represent nagative values.
        let lhs_mant_abs = || lhs_mant.absolute();
        let lhs_mant_f = || lhs_sign.current().select(0.expr() - lhs_mant_abs(), lhs_mant_abs());
        let rhs_mant_abs = || rhs_mant.absolute();
        let rhs_mant_f = || rhs_sign.current().select(0.expr() - rhs_mant_abs(), rhs_mant_abs());
        let out_mant_abs = || out_mant.absolute();
        let out_mant_f = || out_sign.current().select(0.expr() - out_mant_abs(), out_mant_abs());

        let first_exp = || lhs_exp_ge.current().select(lhs_exp.current(), rhs_exp.current());
        let second_exp = || lhs_exp_ge.current().select(rhs_exp.current(), lhs_exp.current());
        let first_mant = || lhs_exp_ge.current().select(lhs_mant_f(), rhs_mant_f());
        let second_mant = || lhs_exp_ge.current().select(rhs_mant_f(), lhs_mant_f());

        let mut opt_lt_gadget = None;

        // Exps is checked to be bytes, so we get negative in any case if exp is not indicate nan or inf.
        // So finally we adding negatives, and it is imposible to wrap modulo.
        // Consequence is than we can add zero to indicate success.
        // let inf_or_nan_arg_exp_case_zero = ||
        //  (lhs_exp.current() - 255.expr()) * (rhs_exp.current() - 255.expr()) + (out_exp.current() - 255.expr());

        cb.condition(lhs_exp.is_inf_or_nan().or(rhs_exp.is_inf_or_nan()).into(), |cb| {
            let out_inf_or_nan: Query<F> = out_exp.is_inf_or_nan().into();
            cb.require_zero("if any argument is inf or nan, than result must be inf of nan",
               1.expr() - out_inf_or_nan,
            );
        });

        let is_nan = || {
            lhs_exp.is_inf_or_nan().and(
                lhs_mant.is_zero(lhs_exp.is_extra_bit_set().into()).not()
            ).or(
                rhs_exp.is_inf_or_nan().and(
                    rhs_mant.is_zero(rhs_exp.is_extra_bit_set().into()).not()
                )
            )
        };

        cb.condition(is_nan().into(), |cb| {
            cb.require_zero("if arg is nan, than out must be nan, so out_mant must not be zero",
                out_mant.is_zero(out_exp.is_extra_bit_set().into()).into()
            );
        });

        // Same sign case.
        cb.condition(is_nan().not().and(lhs_sign.current().xnor(rhs_sign.current())).into(), |cb| {
            let is_inf = || lhs_exp.is_inf_or_nan().and(rhs_exp.is_inf_or_nan()).and(out_exp.is_inf_or_nan());
            let is_denorm_grow = || {
                lhs_exp.is_extra_bit_set().not().or(
                    rhs_exp.is_extra_bit_set().not()
                ).and(
                        out_exp.is_extra_bit_set()
                )
            };
            let out_exp_fix = || is_inf().select(256.expr(), out_exp.current());
            let growing_exp = || out_exp_fix() - first_exp();
            let flip_sign = || 1.expr() - lhs_sign_bit() * 2.expr();
            //let grow = || (growing_exp() + 1.expr());
            let grow = || is_denorm_grow().select(1.expr(), growing_exp() + 1.expr());
            cb.require_boolean("is sign is same, `exp` can grow by zero or one", growing_exp());
            // If `out_exp` is growing, than second mant must satisfy.
            // Second mantissa for check is already truncated.
            let second_mant_for_check = || (out_mant_f() * grow() - growing_exp() * flip_sign() - first_mant()) * pow2.current();
            cb.fixed_lookup(FixedTableTag::Pow2SaturateTo24, [
                is_denorm_grow().select(0.expr(), first_exp() - second_exp()),
                pow2.current(),
                0.expr(),
            ]);
            let dif = || (second_mant() - second_mant_for_check()) * flip_sign();
            let lt_gadget = cb.lt_gadget(pow2.current(), dif());
            cb.require_zero("rest of mantissa must be shifted out of bits", lt_gadget.expr());
            opt_lt_gadget = Some(lt_gadget);
        });

        Self {
            lhs_exp_ge,
            out_exp_ge,
            lhs_sign,
            rhs_sign,
            out_sign,
            lhs_exp,
            rhs_exp,
            out_exp,
            lhs_mant,
            rhs_mant,
            out_mant,
            pow2,
            lt_gadget: opt_lt_gadget,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let lhs = trace.curr_nth_stack_value(1)?;
        let rhs = trace.curr_nth_stack_value(0)?;
        let out = trace.next_nth_stack_value(0)?;

        let lhs_raw = lhs.to_bits() as u32;
        let rhs_raw = rhs.to_bits() as u32;
        let out_raw = out.to_bits() as u32;

        let lhs_sign = (lhs_raw as u64 >> 31) == 1;
        let rhs_sign = (rhs_raw as u64 >> 31) == 1;
        let out_sign = (out_raw as u64 >> 31) == 1;

        let lhs_exp = (lhs_raw >> 23) & 0xff;
        let rhs_exp = (rhs_raw >> 23) & 0xff;
        let out_exp = (out_raw >> 23) & 0xff;

        let lhs_exp_ge = lhs_exp >= rhs_exp;
        self.lhs_exp_ge.assign(region, offset, lhs_exp_ge);

        self.lhs_sign.assign(region, offset, lhs_sign);
        self.rhs_sign.assign(region, offset, rhs_sign);
        self.out_sign.assign(region, offset, out_sign);

        self.lhs_exp.assign(region, offset, F::from(lhs_exp as u64));
        self.rhs_exp.assign(region, offset, F::from(rhs_exp as u64));
        self.out_exp.assign(region, offset, F::from(out_exp as u64));

        let (lhs_mant_raw, lhs_mant_abs) = self.lhs_mant.assign_from_raw(region, offset, lhs_raw);
        let (rhs_mant_raw, rhs_mant_abs) = self.rhs_mant.assign_from_raw(region, offset, rhs_raw);
        let (out_mant_raw, out_mant_abs) = self.out_mant.assign_from_raw(region, offset, out_raw);

        let lhs_mant = if lhs_sign { -(lhs_mant_abs as i64) } else { lhs_mant_abs as i64 };
        let rhs_mant = if rhs_sign { -(rhs_mant_abs as i64) } else { rhs_mant_abs as i64 };
        let out_mant = if out_sign { -(out_mant_abs as i64) } else { out_mant_abs as i64 };

        let first_sign = if lhs_exp_ge { lhs_sign } else { rhs_sign };
        let second_sign = if lhs_exp_ge { rhs_sign } else { lhs_sign };

        let first_exp = if lhs_exp_ge { lhs_exp } else { rhs_exp };
        let second_exp = if lhs_exp_ge { rhs_exp } else { lhs_exp };

        let first_mant = if lhs_exp_ge { lhs_mant } else { rhs_mant };
        let second_mant = if lhs_exp_ge { rhs_mant } else { lhs_mant };

        let out_exp_ge = out_exp >= second_exp;
        self.out_exp_ge.assign(region, offset, out_exp_ge);

        let beta_exp = if out_exp_ge { out_exp } else { second_exp };
        let beta_mant = if out_exp_ge { out_mant } else { second_mant };

        let gamma_sign = if out_exp_ge { second_sign } else { out_sign };
        let gamma_exp = if out_exp_ge { second_exp } else { out_exp };
        let gamma_mant = if out_exp_ge { second_mant } else { out_mant };

        println!("\nFIRST_EXP {}", first_exp);
        println!("SECOND_EXP {}", second_exp);
        println!("BETA_EXP {}", beta_exp);
        println!("GAMMA_EXP {}", gamma_exp);
        println!("OUT_EXP {}", out_exp);
        println!("FIRST_MANT {}", first_mant);
        println!("SECOND_MANT {}", second_mant);
        println!("BETA_MANT {}", beta_mant);
        println!("GAMMA_MANT {}", gamma_mant);
        println!("OUT_MANT {}", out_mant);

        let is_nan = (lhs_exp == 255 && lhs_mant_raw != 0) || (rhs_exp == 255 && rhs_mant_raw != 0);

        if !is_nan && lhs_sign == rhs_sign {

            let out_exp_fix = if lhs_exp == 255 && rhs_exp == 255 && out_exp == 255 { 256 } else { out_exp };
            let is_denorm_grow = (lhs_exp == 0 || rhs_exp == 0) && out_exp != 0;
            let growing_exp = out_exp_fix - first_exp;
            assert!(growing_exp >= 0 && growing_exp <= 1);
            let to_shift = if is_denorm_grow { 0 } else { (first_exp - second_exp).min(24) };
            let pow2 = 1 << to_shift;
            self.pow2.assign(region, offset, F::from(pow2 as u64));
            //let grow = growing_exp as i64 + 1_i64;
            let grow = if is_denorm_grow { 1_i64 } else { growing_exp as i64 + 1_i64 };
            let flip_sign = 1_i64 - lhs_sign as i64 * 2_i64;
            let second_mant_for_check = (out_mant * grow - growing_exp as i64 * flip_sign - first_mant) * pow2 as i64;
            let dif = (second_mant - second_mant_for_check) * flip_sign;
            println!("TO_SHIFT {}", to_shift);
            println!("OUT_MANT GROW {}", out_mant * (growing_exp as i64 + 1_i64));
            println!("SMFC {}", second_mant_for_check);
            println!("GROWING_EXP {}", growing_exp);
            println!("POW2 {}", pow2);
            println!("DIF {}", dif);
            self.lt_gadget.as_ref().unwrap().assign(
                region,
                offset,
                F::from(pow2 as u64),
                F::from(dif as u64),
            );

        }

        if !is_nan && lhs_sign != rhs_sign {
            let first_exp_fix = if lhs_exp == 255 && rhs_exp == 255 && out_exp == 255 { 256 } else { first_exp };
            let growing_exp = first_exp_fix - beta_exp;
            assert!(growing_exp >= 0 && growing_exp <= 1);
            let to_shift = (beta_exp - gamma_exp).min(24);
            let pow2 = 1 << to_shift;
            self.pow2.assign(region, offset, F::from(pow2 as u64));
            let grow = growing_exp as i64 + 1_i64;
            let flip_sign = 1_i64 - gamma_sign as i64 * 2_i64;
            let gamma_mant_for_check = (first_mant * grow - growing_exp as i64 * flip_sign + beta_mant) * pow2 as i64;
            let dif = (gamma_mant - gamma_mant_for_check) * flip_sign;
            println!("TO_SHIFT {}", to_shift);
            println!("FIRST_MANT GROW {}", first_mant * (growing_exp as i64 + 1_i64));
            println!("GMFC {}", gamma_mant_for_check);
            println!("GROWING_EXP {}", growing_exp);
            println!("POW2 {}", pow2);
            println!("DIF {}", dif);
            self.lt_gadget.as_ref().unwrap().assign(
                region,
                offset,
                F::from(pow2 as u64),
                F::from(dif as u64),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    // using https://www.h-schmidt.net/FloatConverter/IEEE754.html.

    #[test]
    fn test_f32_add_extra_bit_not_shifted_out() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x0d00001a) // 0_00011010__0000000_00000000_00011010, 3.94431675125e-31
                                 //            ^                    ^^^^^
                                 //            extra bit            shifted out (number 26)
                                 //                                 so our `dif` is 26
                                 //                                 this is less than 2^11, so it is really shifted out
            F32Add
            // Out   0x12801025     0_00100101__0000000_00010000_00100101, 8.08191560369e-28
            //                                             ^
            //                                             extra bit in result, shifted by 11
            //                                             exp: 37 - 26 = 11
            Drop
        });
    }

    #[test]
    fn test_f32_add_partial_shift_out() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x0d38001a) // 0_00011010__0111000_00000000_00011010, 5.66994998142e-31
                                 //            ^                    ^^^^^
                                 //            extra bit            shifted out (number 26)
                                 //                                 so our `dif` is 26
                                 //                                 this is less than 2^11, so it is really shifted out
            F32Add
            // Out   0x12801725     0_00100101__0000000_00010111_00100101, 8.08364123692e-28
            //                                             ^
            //                                             extra bit in result, shifted by 11
            //                                             exp: 37 - 26 = 11
            Drop
        });
    }

    #[test]
    fn test_f32_add_interfere() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x0d39001a) // 0_00011010__0111001_00000000_00011010, 5.66994998142e-31
                                 //            ^                    ^^^^^
                                 //            extra bit            shifted out (number 26)
                                 //                                 so our `dif` is 26
                                 //                                 this is less than 2^11, so it is really shifted out
            F32Add
            // Out   0x12801745     0_00100101__0000000_00010111_01000101, 8.0836720518e-28
            //                                             ^
            //                                             extra bit in result, shifted by 11
            //                                             exp: 37 - 26 = 11
            Drop
        });
    }

    #[test]
    fn test_f32_add_interfere_order() {
        test_ok(instruction_set! {
            I32Const(0x0d39001a)
            I32Const(0x12800025)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_doubling() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x12800025)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_doubling_neg() {
        test_ok(instruction_set! {
            I32Const(0x92800025_u64) // 1_00100101__0000000_00000000_00100101, -8.07797129917e-28
            I32Const(0x92800025_u64)
            F32Add
            // Out   0x93000025         1_00100110__0000000_00000000_00100101, -1.61559425983e-27
            Drop
        });
    }

    #[test]
    fn test_f32_add_doubling_neg_first_out_pos() {
        test_ok(instruction_set! {
            I32Const(0x92800025_u64)
            I32Const(0x12800026_u64)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_doubling_neg_first_out_neg() {
        test_ok(instruction_set! {
            I32Const(0x92800026_u64)
            I32Const(0x12800025_u64)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_growing_exp_order_first() {
        test_ok(instruction_set! {
            I32Const(0x12800026)
            I32Const(0x12800025)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_growing_exp_order_second() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x12800026)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_neg_growing_exp_order_first() {
        test_ok(instruction_set! {
            I32Const(0x92800025_u64)
            I32Const(0x92800026_u64)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_neg_growing_exp_order_second() {
        test_ok(instruction_set! {
            I32Const(0x92800026_u64)
            I32Const(0x92800025_u64)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_neg_growing_exp_order_second_and_bigger() {
        test_ok(instruction_set! {
            I32Const(0x92800126_u64)
            I32Const(0x92800025_u64)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_smallest_normalized_rhs() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x00800000)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_smallest_normalized_lhs() {
        test_ok(instruction_set! {
            I32Const(0x00800000)
            I32Const(0x12800025)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_exp_127_rhs() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x7f000000)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_inf_rhs() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x7f800000)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_inf_doublind() {
        test_ok(instruction_set! {
            I32Const(0x7f800000)
            I32Const(0x7f800000)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_nan_rhs() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x7f800015)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_denorm() {
        test_ok(instruction_set! {
            I32Const(0x00000015)
            I32Const(0x00000016)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_denorm_out_norm() {
        test_ok(instruction_set! {
            I32Const(0x007fffff)
            I32Const(0x00000001)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_norm_and_denorm_out_norm_simple() {
        test_ok(instruction_set! {
            I32Const(0x00800000)
            I32Const(0x00000010)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_norm_and_denorm_out_norm() {
        test_ok(instruction_set! {
            I32Const(0x12800025)
            I32Const(0x007fffff)
            F32Add
            Drop
        });
    }

    #[test]
    fn test_f32_add_norm_and_denorm_out_norm_special() {
        test_ok(instruction_set! {
            I32Const(0x00800000)
            I32Const(0x007fffff)
            F32Add
            Drop
        });
    }


}
