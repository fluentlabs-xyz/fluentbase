use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
        utils::{
            alloc_u64_with_flag_bit_cell_dyn,
            prepare_alloc_u64_cell,
            AllocatedU64Cell,
            AllocatedU64CellWithFlagBitDyn,
            ShiftOp,
        },
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{arithmetic::FieldExt, circuit::Region};
use num_bigint::BigUint;

#[derive(Clone, Debug)]
pub struct OpShiftGadget<F: Field> {
    lhs: AllocatedU64CellWithFlagBitDyn<F>,
    rhs: AllocatedU64Cell<F>,
    round: AllocatedU64Cell<F>,
    rem: AllocatedU64Cell<F>,
    diff: AllocatedU64Cell<F>,
    pad: AdviceColumn,
    res: AdviceColumn,
    rhs_modulus: AdviceColumn,
    size_modulus: AdviceColumn,

    rhs_round: AdviceColumn,
    rhs_rem: AdviceColumn,
    rhs_rem_diff: AdviceColumn,

    is_i32: SelectorColumn,

    is_shl: SelectorColumn,
    is_shr_u: SelectorColumn,
    is_shr_s: SelectorColumn,
    is_rotl: SelectorColumn,
    is_rotr: SelectorColumn,
    is_l: SelectorColumn,
    is_r: SelectorColumn,

    degree_helper: AdviceColumn,
    // lookup_pow_modulus: AdviceColumn,
    // lookup_pow_power: AdviceColumn,
    // memory_table_lookup_stack_read_lhs: AllocatedMemoryTableLookupReadCell<F>,
    // memory_table_lookup_stack_read_rhs: AllocatedMemoryTableLookupReadCell<F>,
    // memory_table_lookup_stack_write: AllocatedMemoryTableLookupWriteCell<F>,
}

impl<F: Field> ExecutionGadget<F> for OpShiftGadget<F> {
    const NAME: &'static str = "WASM_SHIFT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_SHIFT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32 = cb.query_selector();
        let lhs = alloc_u64_with_flag_bit_cell_dyn(cb, is_i32);
        let rhs = prepare_alloc_u64_cell(cb, Query::one());
        let round = prepare_alloc_u64_cell(cb, Query::one());
        let rem = prepare_alloc_u64_cell(cb, Query::one());
        let diff = prepare_alloc_u64_cell(cb, Query::one());
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

        // TODO query proper
        let res = cb.query_cell();

        cb.stack_pop(rhs.u64_cell.current());
        cb.stack_pop(lhs.u64_cell.current());
        cb.stack_push(res.current());

        // let lookup_pow_modulus = common_config.pow_table_lookup_modulus_cell;
        // let lookup_pow_power = common_config.pow_table_lookup_power_cell;

        // let eid = common_config.eid_cell;
        // let sp = common_config.sp_cell;

        // let memory_table_lookup_stack_read_rhs = allocator.alloc_memory_table_lookup_read_cell(
        //     "op_test stack read",
        //     constraint_builder,
        //     eid,
        //     move |____| Query::from(LocationType::Stack as u64),
        //     move |meta| sp.expr(meta) + Query::from(1),
        //     move |meta| is_i32.expr(meta),
        //     move |meta| rhs.u64_cell.expr(meta),
        //     move |____| Query::from(1),
        // );

        // let memory_table_lookup_stack_read_lhs = allocator.alloc_memory_table_lookup_read_cell(
        //     "op_test stack read",
        //     constraint_builder,
        //     eid,
        //     move |____| Query::from(LocationType::Stack as u64),
        //     move |meta| sp.expr(meta) + Query::from(2),
        //     move |meta| is_i32.expr(meta),
        //     move |meta| lhs.u64_cell.expr(meta),
        //     move |____| Query::from(1),
        // );

        // let memory_table_lookup_stack_write = allocator
        //     .alloc_memory_table_lookup_write_cell_with_value(
        //         "op_test stack read",
        //         constraint_builder,
        //         eid,
        //         move |____| Query::from(LocationType::Stack as u64),
        //         move |meta| sp.expr(meta) + Query::from(2),
        //         move |meta| is_i32.expr(meta),
        //         move |____| Query::from(1),
        //     );
        // let res = memory_table_lookup_stack_write.value_cell;

        cb.require_zeros(
            "bin_shift op select",
            vec![
                is_shr_u.current().0 + is_shr_s.current().0 + is_rotr.current().0
                    - is_r.current().0,
                is_shl.current().0 + is_rotl.current().0 - is_l.current().0,
                is_l.current().0 + is_r.current().0 - Query::from(1),
            ],
        );

        // cs 1: rhs_modulus = if is_i32 { 32 } else { 64 }
        // cs 2: size_modulus = 1 << rhs_modulus
        cb.require_zeros(
            "bin_shift modulus",
            /* /* Box::new(move |meta| */ */
            {
                vec![
                    rhs_modulus.current() - Query::from(64) + is_i32.current().0 * Query::from(32),
                    size_modulus.current() - Query::from_bn(&(BigUint::from(1u64) << 64usize))
                        + is_i32.current().0 * Query::from((u32::MAX as u64) << 32),
                ]
            }, /* ) */
        );

        // cs 3: (rhs_round, rhs_rem) = (rhs & 0xffff) div rhs_modulus
        // cs 3.helper: rhs_rem < rhs_modulus
        cb.require_zeros("bin_shift rhs rem", /* Box::new(move |meta| */ {
            vec![
                rhs_round.current() * rhs_modulus.current() + rhs_rem.current()
                    - rhs.u16_cells_le[0].current(),
                rhs_rem.current() + rhs_rem_diff.current() + Query::from(1) - rhs_modulus.current(),
            ]
        });

        // cs 4: lookup_pow_modulus = 1 << rhs_rem
        // cb.require_zeros(
        //     "bin_shift modulus pow lookup",
        //     /* Box::new(move |meta| */ {
        //         vec![lookup_pow_power.current() - pow_table_power_encode(rhs_rem.current())]
        //     },
        // );

        // cs is_r:
        // 1: (round, rem) = lhs div lookup_pow_modulus
        // 1.helper: rem < lookup_pow_modulus
        // cb.require_zeros(
        //     "bin_shift is_r",
        //     /* Box::new(move |meta| */ {
        //         vec![
        //             is_r.current()
        //                 * (rem.u64_cell.current()
        //                     + round.u64_cell.current() * lookup_pow_modulus.current()
        //                     - lhs.u64_cell.current()),
        //             is_r.current()
        //                 * (rem.u64_cell.current() + diff.u64_cell.current() + Query::from(1)
        //                     - lookup_pow_modulus.current()),
        //         ]
        //     },
        // );

        // cs is_shr_u:
        // 2: res = round
        // cb.require_zeros(
        //     "bin_shift shr_u",
        //     /* Box::new(move |meta| */ {
        //         vec![is_shr_u.current() * (res.current() - round.u64_cell.current())]
        //     },
        // );

        // cs is_shr_s:
        // let size = if is_i32 { 32 } else { 64 }
        // 1. pad = flag * ((1 << rhs_rem) - 1)) << (size - rhs_rem)
        // 2: res = pad + round
        // cb.require_zeros(
        //     "bin_shift shr_s",
        //     /* Box::new(move |meta| */ {
        //         vec![
        //             degree_helper.current()
        //                 - (lookup_pow_modulus.current() - Query::from(1)) *
        //                   size_modulus.current(),
        //             is_shr_s.current()
        //                 * (pad.current() * lookup_pow_modulus.current()
        //                     - lhs.flag_bit_cell.current() * degree_helper.current()),
        //             is_shr_s.current() * (res.current() - round.u64_cell.current() -
        // pad.current()),         ]
        //     },
        // );

        // cs is_rotr:
        // 1: res = round + rem * size_modulus / lookup_pow_modulus
        // cb.require_zeros(
        //     "bin_shift rotr",
        //     /* Box::new(move |meta| */ {
        //         vec![
        //             is_rotr.current()
        //                 * (res.current() * lookup_pow_modulus.current()
        //                     - round.u64_cell.current() * lookup_pow_modulus.current()
        //                     - rem.u64_cell.current() * size_modulus.current()),
        //         ]
        //     },
        // );

        // cs is_l:
        // 1: (round, rem) = (lhs << rhs_rem) div size_modulus
        // 1.helper: rem < size_modulus
        // cb.require_zeros(
        //     "bin_shift shl",
        //     /* Box::new(move |meta| */ {
        //         vec![
        //             is_l.current()
        //                 * (lhs.u64_cell.current() * lookup_pow_modulus.current()
        //                     - round.u64_cell.current() * size_modulus.current()
        //                     - rem.u64_cell.current()),
        //             is_l.current()
        //                 * (rem.u64_cell.current() + diff.u64_cell.current() + Query::from(1)
        //                     - size_modulus.current()),
        //         ]
        //     },
        // );

        // cs is_shl:
        // 1: res = rem
        // cb.require_zeros(
        //     "bin_shift shl",
        //     /* Box::new(move |meta| */ vec![is_shl.current() * (res.current() -
        // rem.u64_cell.current())]), );

        // cs is_rotl:
        // 2: res = rem + round
        // cb.require_zeros(
        //     "bin_shift rotl",
        //     /* Box::new(move |meta| */ {
        //         vec![
        //             is_rotl.current()
        //                 * (res.current() - rem.u64_cell.current() - round.u64_cell.current()),
        //         ]
        //     }),
        // );

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
            is_i32,
            is_shl,
            is_shr_u,
            is_shr_s,
            is_rotl,
            is_rotr,
            is_l,
            is_r,
            // lookup_pow_modulus,
            // lookup_pow_power,
            // memory_table_lookup_stack_read_lhs,
            // memory_table_lookup_stack_read_rhs,
            // memory_table_lookup_stack_write,
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
        let right = trace.curr_nth_stack_value(0)?.to_bits(); // shift
        let left = trace.curr_nth_stack_value(1)?.to_bits(); // value
        let value = trace.next_nth_stack_value(0)?.to_bits(); // res

        let opcode = trace.trace.opcode;
        let (class, left, right, value, power, is_eight_bytes, _is_sign) = match opcode {
            Instruction::I32Rotr
            | Instruction::I32Rotl
            | Instruction::I32Shl
            | Instruction::I32ShrU
            | Instruction::I32ShrS => {
                let left = left as u32 as u64;
                let right = right as u32 as u64;
                let value = value as u32 as u64;
                let power = right % 32;
                let is_eight_bytes = false;
                let is_sign = true;
                let class = OpShiftGadget::<F>::opcode_class(&opcode);
                (class, left, right, value, power, is_eight_bytes, is_sign)
            }
            Instruction::I64Rotl
            | Instruction::I64Rotr
            | Instruction::I64Shl
            | Instruction::I64ShrS
            | Instruction::I64ShrU => {
                let left = left;
                let right = right;
                let value = value;
                let power = right % 64;
                let is_eight_bytes = true;
                let is_sign = true;
                let class = OpShiftGadget::<F>::opcode_class(&opcode);
                (class, left, right, value, power, is_eight_bytes, is_sign)
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
        // self.lookup_pow_modulus
        //     .assign(region, offset, modulus.into())?;
        // self.lookup_pow_power
        //     .assign(ctx, &pow_table_power_encode(BigUint::from(power)))?;
        self.is_i32
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

        // self.memory_table_lookup_stack_read_rhs.assign(
        //     ctx,
        //     entry.memory_rw_entires[0].start_eid,
        //     step.current.eid,
        //     entry.memory_rw_entires[0].end_eid,
        //     step.current.sp + 1,
        //     LocationType::Stack,
        //     !is_eight_bytes,
        //     right,
        // )?;
        //
        // self.memory_table_lookup_stack_read_lhs.assign(
        //     ctx,
        //     entry.memory_rw_entires[1].start_eid,
        //     step.current.eid,
        //     entry.memory_rw_entires[1].end_eid,
        //     step.current.sp + 2,
        //     LocationType::Stack,
        //     !is_eight_bytes,
        //     left,
        // )?;
        //
        // self.memory_table_lookup_stack_write.assign(
        //     ctx,
        //     step.current.eid,
        //     entry.memory_rw_entires[2].end_eid,
        //     step.current.sp + 2,
        //     LocationType::Stack,
        //     !is_eight_bytes,
        //     value,
        // )?;

        Ok(())
    }

    // fn mops(&self, _meta: &mut VirtualCells<'_, F>) -> Option<Expression<F>> {
    //     Some(Query::from(1))
    // }
    //
    // fn memory_writing_ops(&self, _: &EventTableEntry) -> u32 {
    //     1
    // }
    //
    // fn sp_diff(&self, _meta: &mut VirtualCells<'_, F>) -> Option<Expression<F>> {
    //     Some(constant!(F::one()))
    // }
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

    // fn opcode(&self, meta: &mut VirtualCells<'_, F>) -> Expression<F> {
    //     constant!(bn_to_field(
    //         &(BigUint::from(OpcodeClass::BinShift as u64) << OPCODE_CLASS_SHIFT)
    //     )) + self.is_shl.current()
    //         * constant!(bn_to_field( &(BigUint::from(ShiftOp::Shl as u64) << OPCODE_ARG0_SHIFT)
    //         ))
    //         + self.is_shr_u.current()
    //             * constant!(bn_to_field( &(BigUint::from(ShiftOp::UnsignedShr as u64) <<
    //               OPCODE_ARG0_SHIFT)
    //             ))
    //         + self.is_shr_s.current()
    //             * constant!(bn_to_field( &(BigUint::from(ShiftOp::SignedShr as u64) <<
    //               OPCODE_ARG0_SHIFT)
    //             ))
    //         + self.is_rotl.current()
    //             * constant!(bn_to_field( &(BigUint::from(ShiftOp::Rotl as u64) <<
    //               OPCODE_ARG0_SHIFT)
    //             ))
    //         + self.is_rotr.current()
    //             * constant!(bn_to_field( &(BigUint::from(ShiftOp::Rotr as u64) <<
    //               OPCODE_ARG0_SHIFT)
    //             ))
    //         + self.is_i32.current()
    //             * constant!(bn_to_field(&(BigUint::from(1u64) << OPCODE_ARG1_SHIFT)))
    // }
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
