use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn},
    exec_step::{ExecStep, GadgetError},
    fixed_table::FixedTableTag,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
        utils::{
            query_u64_cell,
            query_u64_with_flag_bit_cell_dyn,
            ShiftOp,
            U64Cell,
            U64CellWithFlagBitDyn,
        },
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use num_bigint::BigUint;

const LIMBS_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub struct OpShiftGadget<F: Field> {
    lhs: U64CellWithFlagBitDyn<F>,
    rhs: U64Cell<F>,
    round: U64Cell<F>,
    rem: U64Cell<F>,
    diff: U64Cell<F>,
    pad: AdviceColumn,
    res: AdviceColumn,
    rhs_modulus: AdviceColumn,
    size_modulus: AdviceColumn,

    rhs_round: AdviceColumn,
    rhs_rem: AdviceColumn,
    rhs_rem_diff: AdviceColumn,

    is_i32_otherwise_i64: SelectorColumn,

    is_shl: SelectorColumn,
    is_shr_u: SelectorColumn,
    is_shr_s: SelectorColumn,
    is_rotl: SelectorColumn,
    is_rotr: SelectorColumn,
    is_l: SelectorColumn,
    is_r: SelectorColumn,

    degree_helper: AdviceColumn,
    pow_modulus: AdviceColumn,
    pow_power: AdviceColumn,
}

impl<F: Field> ExecutionGadget<F> for OpShiftGadget<F> {
    const NAME: &'static str = "WASM_SHIFT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_SHIFT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32_otherwise_i64 = cb.query_selector();
        let lhs = query_u64_with_flag_bit_cell_dyn(cb, is_i32_otherwise_i64);
        let rhs = query_u64_cell(cb, Query::one());
        let res = cb.query_cell();
        let round = query_u64_cell(cb, Query::one());
        let rem = query_u64_cell(cb, Query::one());
        let diff = query_u64_cell(cb, Query::one());
        let pad = cb.query_cell();
        let rhs_modulus = cb.query_cell();
        let size_modulus = cb.query_cell();

        let rhs_round = cb.query_cell();
        let rhs_rem = cb.query_cell();
        let rhs_rem_diff = cb.query_cell();

        let is_shl = cb.query_selector();
        let is_shr_u = cb.query_selector();
        let is_shr_s = cb.query_selector();
        let is_rotl = cb.query_selector();
        let is_rotr = cb.query_selector();

        let is_l = cb.query_selector();
        let is_r = cb.query_selector();

        let degree_helper = cb.query_cell();

        cb.stack_pop(rhs.u64.current());
        cb.stack_pop(lhs.u64.current());
        cb.stack_push(res.current());

        let pow_modulus = cb.query_cell();
        let pow_power = cb.query_cell();

        cb.fixed_lookup(
            FixedTableTag::Pow2,
            [pow_power.current(), pow_modulus.current(), Query::zero()],
        );

        cb.require_zeros(
            "op_bin_shift op select",
            vec![
                is_shr_u.current().0 + is_shr_s.current().0 + is_rotr.current().0
                    - is_r.current().0,
                is_shl.current().0 + is_rotl.current().0 - is_l.current().0,
                is_l.current().0 + is_r.current().0 - Query::from(1),
            ],
        );

        // cs 1: rhs_modulus = if is_i32_otherwise_i64 { 32 } else { 64 }
        // cs 2: size_modulus = 1 << rhs_modulus
        cb.require_zeros("op_bin_shift modulus", {
            vec![
                rhs_modulus.current() - Query::from(64)
                    + is_i32_otherwise_i64.current().0 * Query::from(32),
                size_modulus.current() - Query::from_bn(&(BigUint::from(1u64) << 64usize))
                    + is_i32_otherwise_i64.current().0 * Query::from((u32::MAX as u64) << 32),
            ]
        });

        // cs 3: (rhs_round, rhs_rem) = (rhs & 0xffff) div rhs_modulus
        // cs 3.helper: rhs_rem < rhs_modulus
        cb.require_zeros("op_bin_shift rhs rem", {
            vec![
                rhs_round.current() * rhs_modulus.current() + rhs_rem.current()
                    - rhs.u64_as_u16_le[0].current(),
                rhs_rem.current() + rhs_rem_diff.current() + Query::from(1) - rhs_modulus.current(),
            ]
        });

        // cs 4: lookup_pow_modulus = 1 << rhs_rem
        cb.require_zeros(
            "op_bin_shift modulus pow lookup",
            vec![pow_power.current() - rhs_rem.current()],
        );

        // cs is_r:
        // 1: (round, rem) = lhs div lookup_pow_modulus
        // 1.helper: rem < lookup_pow_modulus
        cb.require_zeros("op_bin_shift is_r", {
            vec![
                is_r.current().0
                    * (rem.u64.current() + round.u64.current() * pow_modulus.current()
                        - lhs.u64.current()),
                is_r.current().0
                    * (rem.u64.current() + diff.u64.current() + Query::from(1)
                        - pow_modulus.current()),
            ]
        });

        // cs is_shr_u:
        // 2: res = round
        cb.require_zeros("op_bin_shift shr_u", {
            vec![is_shr_u.current().0 * (res.current() - round.u64.current())]
        });

        // cs is_shr_s:
        // let size = if is_i32_otherwise_i64 { 32 } else { 64 }
        // 1. pad = flag * ((1 << rhs_rem) - 1)) << (size - rhs_rem)
        // 2: res = pad + round
        cb.require_zeros("op_bin_shift shr_s", {
            vec![
                degree_helper.current()
                    - (pow_modulus.current() - Query::from(1)) * size_modulus.current(),
                is_shr_s.current().0
                    * (pad.current() * pow_modulus.current()
                        - lhs.sign_bit.current() * degree_helper.current()),
                is_shr_s.current().0 * (res.current() - round.u64.current() - pad.current()),
            ]
        });

        // cs is_rotr:
        // 1: res = round + rem * size_modulus / lookup_pow_modulus
        cb.require_zeros("op_bin_shift rotr", {
            vec![
                is_rotr.current().0
                    * (res.current() * pow_modulus.current()
                        - round.u64.current() * pow_modulus.current()
                        - rem.u64.current() * size_modulus.current()),
            ]
        });

        // cs is_l:
        // 1: (round, rem) = (lhs << rhs_rem) div size_modulus
        // 1.helper: rem < size_modulus
        cb.require_zeros("op_bin_shift shl", {
            vec![
                is_l.current().0
                    * (lhs.u64.current() * pow_modulus.current()
                        - round.u64.current() * size_modulus.current()
                        - rem.u64.current()),
                is_l.current().0
                    * (rem.u64.current() + diff.u64.current() + Query::from(1)
                        - size_modulus.current()),
            ]
        });

        // cs is_shl:
        // 1: res = rem
        cb.require_zeros(
            "op_bin_shift shl",
            vec![is_shl.current().0 * (res.current() - rem.u64.current())],
        );

        // cs is_rotl:
        // 2: res = rem + round
        cb.require_zeros("op_bin_shift rotl", {
            vec![is_rotl.current().0 * (res.current() - rem.u64.current() - round.u64.current())]
        });

        Self {
            lhs,
            rhs,
            round,
            rem,
            diff,
            pad,
            res,
            rhs_round,
            rhs_rem,
            rhs_rem_diff,
            is_i32_otherwise_i64,
            is_shl,
            is_shr_u,
            is_shr_s,
            is_rotl,
            is_rotr,
            is_l,
            is_r,
            pow_modulus,
            pow_power,
            rhs_modulus,
            size_modulus,
            degree_helper,
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

        let opcode = trace.trace.opcode;
        let (class, left, right, value, power, is_eight_bytes) = match opcode {
            Instruction::I32Rotr
            | Instruction::I32Rotl
            | Instruction::I32Shl
            | Instruction::I32ShrU
            | Instruction::I32ShrS => {
                let left = value as u32 as u64;
                let right = shift as u32 as u64;
                let value = res as u32 as u64;
                let power = right % 32;
                let is_eight_bytes = false;
                let class = OpShiftGadget::<F>::opcode_class(&opcode);
                (class, left, right, value, power, is_eight_bytes)
            }
            Instruction::I64Rotl
            | Instruction::I64Rotr
            | Instruction::I64Shl
            | Instruction::I64ShrS
            | Instruction::I64ShrU => {
                let left = value;
                let right = shift;
                let value = res;
                let power = right % 64;
                let is_eight_bytes = true;
                let class = OpShiftGadget::<F>::opcode_class(&opcode);
                (class, left, right, value, power, is_eight_bytes)
            }
            _ => {
                unreachable!("unsupported shift opcode {:?}", opcode)
            }
        };

        let size = if is_eight_bytes { 64 } else { 32 };
        let size_mask = if is_eight_bytes {
            u64::MAX
        } else {
            u32::MAX as u64
        };

        let modulus = 1u64 << power;
        let size_modulus = if is_eight_bytes {
            BigUint::from(1u64) << 64usize
        } else {
            BigUint::from(1u64) << 32usize
        };

        self.lhs
            .assign(region, offset, left.into(), !is_eight_bytes)?;
        self.rhs.assign(region, offset, right)?;
        self.rhs_round
            .assign(region, offset, (right & 0xffff) / size);
        self.rhs_rem.assign(region, offset, F::from(power));
        self.rhs_rem_diff
            .assign(region, offset, F::from(size - 1 - power));
        self.pow_modulus.assign(region, offset, modulus);
        self.pow_power
            .assign_bn(region, offset, &BigUint::from(power));
        self.is_i32_otherwise_i64
            .assign(region, offset, if is_eight_bytes { false } else { true });
        self.res.assign(region, offset, F::from(value));
        self.rhs_modulus
            .assign(region, offset, if is_eight_bytes { 64 } else { 32 });
        self.size_modulus.assign_bn(region, offset, &size_modulus);
        self.degree_helper
            .assign_bn(region, offset, &(size_modulus * (modulus - 1)));

        match class {
            ShiftOp::Shl => {
                self.is_l.enable(region, offset);
                self.is_shl.enable(region, offset);
                if power != 0 {
                    self.round.assign(region, offset, left >> (size - power))?;
                } else {
                    self.round.assign(region, offset, 0)?;
                }
                let rem = (left << power) & size_mask;
                self.rem.assign(region, offset, rem)?;
                self.diff.assign(region, offset, size_mask - rem)?;
            }
            ShiftOp::UnsignedShr => {
                self.is_r.enable(region, offset);
                self.is_shr_u.enable(region, offset);
                self.round.assign(region, offset, left >> power)?;
                let rem = left & ((1 << power) - 1);
                self.rem.assign(region, offset, rem)?;
                self.diff
                    .assign(region, offset, (1u64 << power) - rem - 1)?;
            }
            ShiftOp::SignedShr => {
                self.is_r.enable(region, offset);
                self.is_shr_s.enable(region, offset);
                self.round.assign(region, offset, left >> power)?;
                let rem = left & ((1 << power) - 1);
                self.rem.assign(region, offset, rem)?;
                self.diff
                    .assign(region, offset, (1u64 << power) - 1 - rem)?;

                let flag_bit = if is_eight_bytes {
                    left >> 63
                } else {
                    left >> 31
                };
                if flag_bit == 1 && power != 0 {
                    self.pad
                        .assign(region, offset, ((1 << power) - 1) << (size - power));
                }
            }
            ShiftOp::Rotl => {
                // same as shl
                self.is_l.enable(region, offset);
                self.is_rotl.enable(region, offset);
                if power != 0 {
                    self.round.assign(region, offset, left >> (size - power))?;
                } else {
                    self.round.assign(region, offset, 0)?;
                }
                let rem = (left << power) & size_mask;
                self.rem.assign(region, offset, rem)?;
                self.diff.assign(region, offset, size_mask - rem)?;
            }
            ShiftOp::Rotr => {
                // same as shr_u
                self.is_r.enable(region, offset);
                self.is_rotr.enable(region, offset);
                self.round.assign(region, offset, left >> power)?;
                let rem = left & ((1 << power) - 1);
                self.rem.assign(region, offset, rem)?;
                self.diff
                    .assign(region, offset, (1u64 << power) - rem - 1)?;
            }
        }

        Ok(())
    }
}

impl<F: Field> OpShiftGadget<F> {
    fn opcode_class(opcode: &Instruction) -> ShiftOp {
        match opcode {
            Instruction::I64Rotl => ShiftOp::Rotl,
            Instruction::I64Rotr => ShiftOp::Rotr,
            Instruction::I64Shl => ShiftOp::Shl,
            Instruction::I64ShrS => ShiftOp::SignedShr,
            Instruction::I64ShrU => ShiftOp::UnsignedShr,
            Instruction::I32Rotr => ShiftOp::Rotr,
            Instruction::I32Rotl => ShiftOp::Rotl,
            Instruction::I32Shl => ShiftOp::Shl,
            Instruction::I32ShrU => ShiftOp::UnsignedShr,
            Instruction::I32ShrS => ShiftOp::SignedShr,
            _ => {
                unreachable!()
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use log::debug;
    use rand::{thread_rng, Rng};

    fn gen_params<const N: usize, const MAX_VAL: i64, const POSITIVE: bool>() -> [i64; N] {
        let params = {
            if POSITIVE {
                [0; N].map(|_| thread_rng().gen_range(0..=MAX_VAL))
            } else {
                [0; N].map(|_| thread_rng().gen_range(0..=MAX_VAL + 1) - (MAX_VAL + 1))
            }
        };
        debug!("params {:?}", params);
        params
    }

    #[test]
    fn test_i32shl() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32Shl

            Drop
        });
    }

    #[test]
    fn test_i32shr_s() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32ShrS

            Drop
        });
    }

    #[test]
    fn test_i32shr_u() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32ShrU

            Drop
        });
    }

    #[test]
    fn test_i64shl() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64Shl

            Drop
        });
    }

    #[test]
    fn test_i64shr_s() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64ShrS

            Drop
        });
    }

    #[test]
    fn test_i64shr_u() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64ShrU

            Drop
        });
    }

    #[test]
    fn test_i32rotl() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32Rotl

            Drop
        });
    }

    #[test]
    fn test_i32rotr() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I32Rotr

            Drop
        });
    }

    #[test]
    fn test_i64rotl() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64Rotl

            Drop
        });
    }

    #[test]
    fn test_i64rotr() {
        let [v] = gen_params::<1, 65535, true>();
        let [s] = gen_params::<1, 65535, true>();
        test_ok(instruction_set! {
            I32Const[v]
            I32Const[s]
            I64Rotr

            Drop
        });
    }
}
