use crate::{
    constraint_builder::ToExpr,
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
pub(crate) struct OpStoreGadget<F> {
    // same_context: SameContextGadget<F>,
    // opcode_store_offset: AdviceColumn,
    //
    // store_start_block_index: AdviceColumn,
    // store_start_block_inner_offset: AdviceColumn,
    // store_start_block_inner_offset_helper: AdviceColumn,
    //
    // store_end_block_index: AdviceColumn,
    // store_end_block_inner_offset: AdviceColumn,
    // store_end_block_inner_offset_helper: AdviceColumn,
    //
    // load_value1: AdviceColumn,
    // load_value2: AdviceColumn,
    // store_value1: AdviceColumn,
    // store_value2: AdviceColumn,
    //
    // mask_bits: [AdviceColumn; 16],
    // offset_modulus: AdviceColumn,
    // store_raw_value: AdviceColumn,
    // store_base: AdviceColumn,
    // store_wrapped_value: AdviceColumn,
    //
    // vtype: AdviceColumn,
    // is_one_byte: AdviceColumn,
    // is_two_bytes: AdviceColumn,
    // is_four_bytes: AdviceColumn,
    // is_eight_bytes: AdviceColumn,
    //
    // //lookup_offset_len_bits: OffsetLenBitsTableLookupCell,
    // //lookup_pow: PowTableLookupCell,
    // address_within_allocated_pages_helper: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpStoreGadget<F> {
    const NAME: &'static str = "WASM_STORE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_STORE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32_store = cb.query_fixed();

        let value = cb.query_cell();
        let address = cb.query_cell();
        let offset = cb.query_cell();

        cb.stack_pop(value.current());
        cb.stack_pop(address.current());

        // Instruction::I32Store(Default::default()),
        // Instruction::I32Store8(Default::default()),
        // Instruction::I32Store16(Default::default()),
        // Instruction::I64Store(Default::default()),
        // Instruction::I64Store8(Default::default()),
        // Instruction::I64Store16(Default::default()),
        // Instruction::I64Store32(Default::default()),
        // Instruction::F32Store(Default::default()),
        // Instruction::F64Store(Default::default()),

        cb.require_at_least_one_selector([is_i32_store.current()]);

        cb.if_rwasm_opcode(
            is_i32_store.current(),
            Instruction::I32Store(Default::default()),
            |cb| {
                (0..4).for_each(|i| {
                    cb.mem_write(
                        address.current() + offset.current() + i.expr(),
                        value.current(),
                    );
                });
            },
        );

        // todo: "and so on"

        // store
        // value=read_stack
        // addr=read_stack
        // mem[addr+offset] = value

        // i32.store{len=4}
        // i32.store16{len=2}
        // i32.store8{len=1}

        // mem[0x00] = 1;
        // let val = mem[0x00]

        // mem[0x00] = 0x7bff;
        // mem[0x01] == 0x7b;

        // i32.store8 <0x7b>

        // i32.store(0) <0x00> <0x00007bff>
        // 00: 0xff
        // 01: 0x7b
        // 02: 0x00
        // 03: 0x00

        // i32.load8S(0) <0x01>

        // let opcode_store_offset = cb.alloc_common_range_value();
        //
        // let store_start_block_index = cb.alloc_common_range_value();
        // let store_start_block_inner_offset = cb.alloc_common_range_value();
        // let store_start_block_inner_offset_helper = cb.alloc_common_range_value();
        //
        // let store_end_block_index = cb.alloc_common_range_value();
        // let store_end_block_inner_offset = cb.alloc_common_range_value();
        // let store_end_block_inner_offset_helper = cb.alloc_common_range_value();
        //
        // let load_value1 = cb.alloc_u64_on_u8();
        // let load_value2 = cb.alloc_u64_on_u8();
        // let store_value1 = cb.alloc_u64_on_u8();
        // let store_value2 = cb.alloc_u64_on_u8();
        // let offset_modulus = cb.alloc_u64();
        // let store_raw_value = cb.alloc_u64();
        // let store_base = cb.alloc_u64();
        //
        // let store_wrapped_value = cb.alloc_unlimited_value();
        //
        // let mask_bits = [0; 16].map(|_| cb.alloc_bit_value());
        // let is_one_byte = cb.alloc_bit_value();
        // let is_two_bytes = cb.alloc_bit_value();
        // let is_four_bytes = cb.alloc_bit_value();
        // let is_eight_bytes = cb.alloc_bit_value();
        // let vtype = cb.alloc_common_range_value();
        //
        // let lookup_offset_len_bits = cb.alloc_offset_len_bits_table_lookup();
        // let lookup_pow = cb.alloc_pow_table_lookup();
        //
        // let current_memory_page_size = cb.allocated_memory_pages_cell();
        // let address_within_allocated_pages_helper = cb.alloc_common_range_value();
        //
        // cb.stack_pop(value.expr());
        // cb.stack_pop(raw_address.expr());
        // cb.stack_pop(pre_block_value.expr());
        // cb.stack_push(update_block_value1.expr());
        //
        // cb.require_zeros(
        //     "op_store: start end offset range",
        //     vec![
        //         store_start_block_inner_offset.expr()
        //             + store_start_block_inner_offset_helper.expr()
        //             - 7.expr(),
        //         store_end_block_inner_offset.expr() + store_end_block_inner_offset_helper.expr()
        //             - 7.expr(),
        //     ],
        // );
        //
        // cb.require_zeros("op_store: start end equation", {
        //     let len = 1.expr()
        //         + is_two_bytes.expr() * 1.expr()
        //         + is_four_bytes.expr() * 3.expr()
        //         + is_eight_bytes.expr() * 7.expr();
        //     vec![
        //         store_start_block_index.expr() * 8.expr()
        //             + store_start_block_inner_offset.expr()
        //             + len
        //             - 1.expr()
        //             - store_end_block_index.expr() * 8.expr()
        //             - store_end_block_inner_offset.expr(),
        //     ]
        // });
        //
        // cb.require_zeros(
        //     "op_store: start store_base",
        //     vec![
        //         store_base.expr() + opcode_store_offset.expr()
        //             - store_start_block_index.expr() * 8.expr()
        //             - store_start_block_inner_offset.expr(),
        //     ],
        // );
        //
        // cb.require_zeros(
        //     "op_store: length",
        //     vec![
        //         is_one_byte.expr()
        //             + is_two_bytes.expr()
        //             + is_four_bytes.expr()
        //             + is_eight_bytes.expr()
        //             - 1.expr(),
        //     ],
        // );
        //
        // cb.require_zeros("op_store: mask_bits offset len", {
        //     let len = 1.expr()
        //         + is_two_bytes.expr() * 1.expr()
        //         + is_four_bytes.expr() * 3.expr()
        //         + is_eight_bytes.expr() * 7.expr();
        //     let (_, bits_encode) = mask_bits
        //         .map(|c| c.expr())
        //         .into_iter()
        //         .enumerate()
        //         .reduce(|(_, acc), (i, e)| (i, acc + e * (1u64 << i).expr()))
        //         .unwrap();
        //     vec![
        //         lookup_offset_len_bits.expr()
        //             - offset_len_bits_encode_expr( store_start_block_inner_offset.expr(), len,
        //               bits_encode,
        //             ),
        //     ]
        // });
        //
        // cb.require_zeros(
        //     "op_store: pow table lookup",
        //     vec![
        //         lookup_pow.expr()
        //             - pow_table_encode( offset_modulus.expr(),
        //               store_start_block_inner_offset.expr() * 8.expr(),
        //             ),
        //     ],
        // );
        //
        // /*constraint_builder.push(
        //     "op_store wrap value",
        //     Box::new(move |meta| {
        //         let has_two_bytes =
        //             is_two_bytes.expr(meta) + is_four_bytes.expr(meta) +
        // is_eight_bytes.expr(meta);         let has_four_bytes = is_four_bytes.expr(meta)
        // + is_eight_bytes.expr(meta);         let has_eight_bytes =
        // is_eight_bytes.expr(meta);         let byte_value = (0..8)
        //             .map(|i| {
        //                 store_raw_value.u4_expr(meta, i * 2) * constant_from!(1u64 << (8 * i))
        //                     + store_raw_value.u4_expr(meta, i * 2 + 1)
        //                         * constant_from!(1u64 << (8 * i + 4))
        //             })
        //             .collect::<Vec<_>>();
        //         vec![
        //             byte_value[0].clone()
        //                 + byte_value[1].clone() * has_two_bytes
        //                 + (byte_value[2].clone() + byte_value[3].clone()) * has_four_bytes
        //                 + (byte_value[4].clone()
        //                     + byte_value[5].clone()
        //                     + byte_value[6].clone()
        //                     + byte_value[7].clone())
        //                     * has_eight_bytes
        //                 - store_wrapped_value.expr(meta),
        //         ]
        //     }),
        // );*/
        //
        // /*constraint_builder.push(
        //     "op_store write value",
        //     Box::new(move |meta| {
        //         let mut acc = store_wrapped_value.expr(meta) * offset_modulus.expr(meta);
        //
        //         for i in 0..8 {
        //             acc = acc
        //                 - store_value1.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8))))
        //                     * mask_bits[i as usize].expr(meta);
        //
        //             acc = acc
        //                 - store_value2.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8 + 64))))
        //                     * mask_bits[i as usize + 8].expr(meta);
        //         }
        //
        //         vec![acc]
        //     }),
        // );*/
        //
        // /*constraint_builder.push(
        //     "op_store unchanged value",
        //     Box::new(move |meta| {
        //         let mut acc = constant_from!(0);
        //
        //         for i in 0..8 {
        //             acc = acc
        //                 + load_value1.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8))))
        //                     * (constant_from!(1) - mask_bits[i as usize].expr(meta))
        //                 - store_value1.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8))))
        //                     * (constant_from!(1) - mask_bits[i as usize].expr(meta));
        //
        //             acc = acc
        //                 + load_value2.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8 + 64))))
        //                     * (constant_from!(1) - mask_bits[i as usize + 8].expr(meta))
        //                 - store_value2.u8_expr(meta, i)
        //                     * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8 + 64))))
        //                     * (constant_from!(1) - mask_bits[i as usize + 8].expr(meta));
        //         }
        //
        //         vec![acc]
        //     }),
        // );*/
        //
        // cb.require_zeros("op_store: allocated address", {
        //     let len = 1.expr()
        //         + is_two_bytes.expr() * 1.expr()
        //         + is_four_bytes.expr() * 3.expr()
        //         + is_eight_bytes.expr() * 7.expr();
        //     vec![
        //         (store_base.expr()
        //             + opcode_store_offset.expr()
        //             + len
        //             + address_within_allocated_pages_helper.expr()
        //             - current_memory_page_size.expr() * WASM_PAGE_SIZE.expr()),
        //     ]
        // });
        //
        // let opcode = cb.query_cell();
        //
        // // State transition
        // let step_state_transition = StepStateTransition {
        //     rw_counter: Delta(4.expr()),
        //     program_counter: Delta(1.expr()),
        //     stack_pointer: Delta(0.expr()),
        //     // TODO: change op.
        //     gas_left: Delta(-OpcodeId::I32Eqz.constant_gas_cost().expr()),
        //     ..StepStateTransition::default()
        // };
        // let same_context = SameContextGadget::construct(cb, opcode, step_state_transition);

        Self {
            // same_context,
            // opcode_store_offset,
            // store_start_block_index,
            // store_start_block_inner_offset,
            // store_start_block_inner_offset_helper,
            // store_end_block_index,
            // store_end_block_inner_offset,
            // store_end_block_inner_offset_helper,
            // store_value1,
            // store_value2,
            // mask_bits,
            // offset_modulus,
            // store_base,
            // store_raw_value,
            // store_wrapped_value,
            // is_one_byte,
            // is_two_bytes,
            // is_four_bytes,
            // is_eight_bytes,
            // vtype,
            // load_value1,
            // load_value2,
            // address_within_allocated_pages_helper,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::I32Store(address_offset) => {
                // enable selector
            }
            Instruction::I32Store8(address_offset) => {}
            Instruction::I32Store16(address_offset) => {}
            Instruction::I64Store(address_offset) => {}
            Instruction::I64Store8(address_offset) => {}
            Instruction::I64Store16(address_offset) => {}
            Instruction::I64Store32(address_offset) => {}
            Instruction::F32Store(address_offset) => {}
            Instruction::F64Store(address_offset) => {}
            _ => unreachable!("illegal opcode place {:?}", trace.instr()),
        }
        // self.same_context.assign_exec_step(region, offset, step)?;

        // let opcode = step.opcode.unwrap();

        // cb.stack_pop(value.expr());
        // cb.stack_pop(raw_address.expr());
        // cb.stack_pop(pre_block_value.expr());
        // cb.stack_push(update_block_value1.expr());

        // let [value, raw_address, pre_block_value, update_block_value1] = [
        //     trace.curr_nth_stack_value(0)?,
        //     trace.curr_nth_stack_value(1)?,
        //     trace.curr_nth_stack_value(2)?,
        //     trace.curr_nth_stack_value(3)?,
        //     // step.rw_indices[0],
        //     // step.rw_indices[1],
        //     // step.rw_indices[2],
        //     // step.rw_indices[3],
        // ];

        /*
                self.value.assign(region, offset, Value::known(value.to_scalar().unwrap()))?;
                self.value_inv.assign(region, offset, Value::known(F::from(value.as_u64()).invert().unwrap_or(F::zero())))?;
                self.res.assign(region, offset, Value::known(res.to_scalar().unwrap()))?;

                match opcode {
                    OpcodeId::I64Eqz => {
                        let zero_or_one = (value.as_u64() == 0) as u64;
                        self.res.assign(region, offset, Value::known(F::from(zero_or_one)))?;
                    }
                    OpcodeId::I32Eqz => {
                        let zero_or_one = (value.as_u32() == 0) as u64;
                        self.res.assign(region, offset, Value::known(F::from(zero_or_one)))?;
                    }
                    _ => unreachable!("not supported opcode: {:?}", opcode),
                };

                let is_i64 = matches!(opcode,
                    OpcodeId::I64Eqz
                );
                self.is_i64.assign(region, offset, Value::known(F::from(is_i64 as u64)))?;
        */

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_i32_store() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            I32Store[0]
        });
    }

    #[test]
    fn test_i32_store8() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            I32Store8[0]
        });
    }

    #[test]
    fn test_i32_store16() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            I32Store16[0]
        });
    }

    #[test]
    fn test_i64_store() {
        test_ok(instruction_set! {
            I64Const[1]
            I64Const[2]
            I64Store[0]
        });
    }

    #[test]
    fn test_i64_store8() {
        test_ok(instruction_set! {
            I64Const[1]
            I64Const[2]
            I64Store8[0]
        });
    }

    #[test]
    fn test_i64_store16() {
        test_ok(instruction_set! {
            I64Const[1]
            I64Const[2]
            I64Store16[0]
        });
    }

    #[test]
    fn test_i64_store32() {
        test_ok(instruction_set! {
            I64Const[1]
            I64Const[2]
            I64Store32[0]
        });
    }

    #[test]
    fn test_f32_store() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            F32Store[0]
        });
    }

    #[test]
    fn test_f64_store() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            F64Store[0]
        });
    }
}
