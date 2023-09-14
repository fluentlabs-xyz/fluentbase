use crate::{
    constraint_builder::{AdviceColumn, FixedColumn},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    rw_builder::copy_row::CopyTableTag,
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpMemoryGadget<F: Field> {
    is_memory_copy: FixedColumn,
    is_memory_fill: FixedColumn,
    is_memory_grow: FixedColumn,
    dest: AdviceColumn,
    source: AdviceColumn,
    len: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemoryGadget<F> {
    const NAME: &'static str = "WASM_MEMORY";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_memory_copy = cb.query_fixed();
        let is_memory_fill = cb.query_fixed();
        let is_memory_grow = cb.query_fixed();

        let dest = cb.query_cell();
        let source = cb.query_cell();
        let len = cb.query_cell();

        cb.stack_pop(len.current());
        cb.stack_pop(source.current());
        cb.stack_pop(dest.current());

        cb.require_exactly_one_selector([
            is_memory_copy.current(),
            is_memory_fill.current(),
            is_memory_grow.current(),
        ]);

        cb.if_rwasm_opcode(is_memory_copy.current(), Instruction::MemoryCopy, |cb| {
            cb.copy_lookup(
                CopyTableTag::CopyMemory,
                source.current(),
                dest.current(),
                len.current(),
            );
        });

        Self {
            is_memory_copy,
            is_memory_fill,
            is_memory_grow,
            dest,
            source,
            len,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::MemoryCopy => {
                self.is_memory_copy.assign(region, offset, 1u64);
                let len = trace.curr_nth_stack_value(0)?;
                let source = trace.curr_nth_stack_value(1)?;
                let dest = trace.curr_nth_stack_value(2)?;
                self.len.assign(region, offset, len.as_u64());
                self.source.assign(region, offset, source.as_u64());
                self.dest.assign(region, offset, dest.as_u64());
            }
            Instruction::MemoryFill => {
                self.is_memory_fill.assign(region, offset, 1u64);
            }
            Instruction::MemoryGrow => {
                self.is_memory_grow.assign(region, offset, 1u64);
            }
            _ => unreachable!("illegal opcode place {:?}", trace.instr()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_copy() {
        let default_memory = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let code = instruction_set! {
            .add_memory(0, default_memory.as_slice())
            I32Const(5)
            I32Const(0)
            I32Const(5)
            MemoryCopy
        };
        test_ok(code);
    }
}
