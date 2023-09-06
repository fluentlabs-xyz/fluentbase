use crate::{
    constraint_builder::{AdviceColumn, SelectorColumn, ToExpr},
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
use num_traits::ToPrimitive;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpStoreGadget<F> {
    is_i32_store: SelectorColumn,
    is_i32_store8: SelectorColumn,
    is_i32_store16: SelectorColumn,
    is_i64_store: SelectorColumn,
    is_i64_store8: SelectorColumn,
    is_i64_store16: SelectorColumn,
    is_i64_store32: SelectorColumn,
    is_f32_store: SelectorColumn,
    is_f64_store: SelectorColumn,

    value: AdviceColumn,
    value_limbs: [AdviceColumn; 8],
    address: AdviceColumn,
    address_base_offset: AdviceColumn,

    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpStoreGadget<F> {
    const NAME: &'static str = "WASM_STORE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_STORE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32_store = cb.query_selector();
        let is_i32_store8 = cb.query_selector();
        let is_i32_store16 = cb.query_selector();
        let is_i64_store = cb.query_selector();
        let is_i64_store8 = cb.query_selector();
        let is_i64_store16 = cb.query_selector();
        let is_i64_store32 = cb.query_selector();
        let is_f32_store = cb.query_selector();
        let is_f64_store = cb.query_selector();

        let value = cb.query_cell();
        let value_limbs = cb.query_cells();
        let address = cb.query_cell();
        let address_base_offset = cb.query_cell();

        cb.stack_pop(value.current());
        cb.stack_pop(address.current());

        cb.require_exactly_one_selector(
            [
                is_i32_store,
                is_i32_store8,
                is_i32_store16,
                is_i64_store,
                is_i64_store8,
                is_i64_store16,
                is_i64_store32,
                is_f32_store,
                is_f64_store,
            ]
            .map(|v| v.current().0),
        );

        cb.if_rwasm_opcode(
            is_i32_store.current().0,
            Instruction::I32Store(Default::default()),
            |cb| {
                (0..4).for_each(|i| {
                    cb.mem_write(
                        address_base_offset.current() + address.current() + i.expr(),
                        value_limbs[i].current(),
                    );
                });
            },
        );

        Self {
            is_i32_store,
            is_i32_store8,
            is_i32_store16,
            is_i64_store,
            is_i64_store8,
            is_i64_store16,
            is_i64_store32,
            is_f32_store,
            is_f64_store,
            value,
            address,
            value_limbs,
            address_base_offset,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let value = trace.curr_nth_stack_value(0)?.to_bits();
        let address = trace.curr_nth_stack_value(1)?.to_bits();
        let value_le_bytes = value.to_le_bytes();

        let mut assign = |selector: &SelectorColumn, length: usize, address_base_offset: u32| {
            selector.enable(region, offset);
            self.value.assign(region, offset, value);
            (0..length).for_each(|i| {
                self.value_limbs[i].assign(region, offset, value_le_bytes[i] as u64);
            });
            self.address.assign(region, offset, address);
            self.address_base_offset
                .assign(region, offset, address_base_offset as u64);
        };

        match trace.instr() {
            Instruction::I32Store(address_base_offset) => {
                assign(&self.is_i32_store, 4, address_base_offset.into_inner());
            }
            Instruction::I32Store8(address_base_offset) => {
                assign(&self.is_i32_store8, 1, address_base_offset.into_inner());
            }
            Instruction::I32Store16(address_base_offset) => {
                assign(&self.is_i32_store16, 2, address_base_offset.into_inner());
            }
            Instruction::I64Store(address_base_offset) => {
                assign(&self.is_i64_store, 8, address_base_offset.into_inner());
            }
            Instruction::I64Store8(address_base_offset) => {
                assign(&self.is_i64_store8, 1, address_base_offset.into_inner());
            }
            Instruction::I64Store16(address_base_offset) => {
                assign(&self.is_i64_store16, 2, address_base_offset.into_inner());
            }
            Instruction::I64Store32(address_base_offset) => {
                assign(&self.is_i64_store32, 4, address_base_offset.into_inner());
            }
            Instruction::F32Store(address_base_offset) => {
                assign(&self.is_f32_store, 4, address_base_offset.into_inner());
            }
            Instruction::F64Store(address_base_offset) => {
                assign(&self.is_f64_store, 8, address_base_offset.into_inner());
            }
            _ => unreachable!("illegal opcode place {:?}", trace.instr()),
        };

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
            I32Const[1] // address
            I32Const[800] // value
            I32Store[0 /*address_base_offset*/]
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
