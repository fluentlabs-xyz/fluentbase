use crate::{
    constraint_builder::{AdviceColumn, Query, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        memory_expansion::MemoryExpansionGadget,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpStoreGadget<F: Field, const N_STORE_BYTES: usize> {
    value: AdviceColumn,
    value_stored: AdviceColumn,
    value_as_bytes: [AdviceColumn; Instruction::MAX_BYTE_LEN],
    address: AdviceColumn,
    address_base_offset: AdviceColumn,
    memory_expansion: MemoryExpansionGadget<F>,
    marker: PhantomData<F>,
}

impl<F: Field, const N_STORE_BYTES: usize> ExecutionGadget<F> for OpStoreGadget<F, N_STORE_BYTES> {
    const NAME: &'static str = "WASM_STORE";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_STORE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();
        let value_stored = cb.query_cell();
        let value_as_bytes = cb.query_cells();
        let address = cb.query_cell();
        let address_base_offset = cb.query_cell();

        value_as_bytes.iter().for_each(|v| {
            cb.range_check8(v.current());
        });

        let memory_expansion = MemoryExpansionGadget::configure(cb);

        cb.stack_pop(value.current());
        cb.stack_pop(address.current());

        let mut value_reconstructed = Query::zero();
        for i in 0..N_STORE_BYTES {
            let i_rev = N_STORE_BYTES - 1 - i;
            value_reconstructed =
                value_reconstructed * Query::from(0x100) + value_as_bytes[i_rev].current();
            cb.mem_write(
                address_base_offset.current() + address.current() + i.expr(),
                value_as_bytes[i].current(),
            );
        }
        cb.require_equal(
            "value_stored=value_reconstructed",
            value_reconstructed,
            value_stored.current(),
        );

        Self {
            value,
            address,
            value_as_bytes,
            address_base_offset,
            value_stored,
            memory_expansion,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &ExecStep,
    ) -> Result<(), GadgetError> {
        let value = step.curr_nth_stack_value(0)?.to_bits();
        let address = step.curr_nth_stack_value(1)?.to_bits();
        let value_le_bytes = value.to_le_bytes();
        let instr = step.instr();
        let address_offset = instr.aux_value().unwrap_or_default();
        let mut value_reconstructed = 0;
        let mut mul = 1;
        let byte_len = Instruction::store_instr_meta(instr);
        (0..byte_len).for_each(|i| {
            if i > 0 {
                mul *= 0x100;
            }
            let val = value_le_bytes[i] as u64;
            value_reconstructed += mul * val;
            self.value_as_bytes[i].assign(region, offset, val);
        });
        self.value.assign(region, offset, value);
        self.value_stored
            .assign(region, offset, value_reconstructed);
        self.address.assign(region, offset, address);
        self.address_base_offset
            .assign(region, offset, address_offset.as_u64());
        self.memory_expansion.assign(region, offset, step);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use rand::{thread_rng, Rng};

    fn gen_address_params() -> [u32; 2] {
        [
            thread_rng().gen_range(0..100),
            thread_rng().gen_range(0..100),
        ]
    }

    #[test]
    fn test_i32_store_with_const_after() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[800]
            I32Store[address_offset]
            I32Const(0)
            Drop
        });
    }

    #[test]
    fn test_i32_store_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[800]
            I32Store[address_offset]
        });
    }

    #[test]
    fn test_i32_store8_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[2]
            I32Store8[address_offset]
        });
    }

    #[test]
    fn test_i32_store16_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[2]
            I32Store16[address_offset]
        });
    }

    #[test]
    fn test_i64_store_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[2]
            I64Store[address_offset]
        });
    }

    #[test]
    fn test_i64_store8_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[2]
            I64Store8[address_offset]
        });
    }

    #[test]
    fn test_i64_store16_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[2]
            I64Store16[address_offset]
        });
    }

    #[test]
    fn test_i64_store32_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[2]
            I64Store32[address_offset]
        });
    }

    #[test]
    #[ignore]
    fn test_f32_store_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[2]
            F32Store[address_offset]
        });
    }

    #[test]
    #[ignore]
    fn test_f64_store_positive_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[2]
            F64Store[address_offset]
        });
    }

    #[test]
    fn test_i32_store_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[-800]
            I32Store[address_offset]
        });
    }

    #[test]
    fn test_i32_store8_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[-2]
            I32Store8[address_offset]
        });
    }

    #[test]
    fn test_i32_store16_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[-800]
            I32Store16[address_offset]
        });
    }

    #[test]
    fn test_i64_store_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[-2]
            I64Store[address_offset]
        });
    }

    #[test]
    fn test_i64_store8_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[-3]
            I64Store8[address_offset]
        });
    }

    #[test]
    fn test_i64_store16_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[-800]
            I64Store16[address_offset]
        });
    }

    #[test]
    fn test_i64_store32_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I64Const[-2]
            I64Store32[address_offset]
        });
    }

    #[test]
    #[ignore]
    fn test_f32_store_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[-2]
            F32Store[address_offset]
        });
    }

    #[test]
    #[ignore]
    fn test_f64_store_negative_number() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            .add_memory(0, &[0; 1024])
            I32Const[address]
            I32Const[-2]
            F64Store[address_offset]
        });
    }
}
