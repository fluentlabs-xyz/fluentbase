use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, FixedColumn},
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, ToExpr},
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{circuit::Region, plonk::Error};
use std::marker::PhantomData;

pub(crate) struct LocalGadget<F: Field> {
    is_get_local: FixedColumn,
    is_set_local: FixedColumn,
    is_tee_local: FixedColumn,
    index: AdviceColumn,
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for LocalGadget<F> {
    const NAME: &'static str = "WASM_LOCAL";

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_get_local = cb.query_fixed();
        let is_set_local = cb.query_fixed();
        let is_tee_local = cb.query_fixed();

        let index = cb.query_cell();
        let value = cb.query_cell();

        cb.require_equal(
            "op_local: selector",
            is_get_local.expr() + is_set_local.expr() + is_tee_local.expr(),
            1.expr(),
        );

        cb.condition(is_set_local.expr(), |cb| {
            cb.stack_pop(value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer_offset() + index.expr(),
                value.expr(),
            );
        });

        cb.condition(is_get_local.expr(), |cb| {
            cb.stack_lookup(
                0.expr(),
                cb.stack_pointer_offset() + index.expr(),
                value.expr(),
            );
            cb.stack_push(value.expr());
        });

        cb.condition(is_tee_local.expr(), |cb| {
            cb.stack_pop(value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer_offset() + index.expr() - 1.expr(),
                value.expr(),
            );
            cb.stack_push(value.expr());
        });

        Self {
            is_set_local,
            is_get_local,
            is_tee_local,
            index,
            value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        instr: Instruction,
    ) -> Result<(), Error> {
        let (selector, index) = match instr {
            Instruction::LocalGet(val) => (&self.is_get_local, val),
            Instruction::LocalSet(val) => (&self.is_set_local, val),
            Instruction::LocalTee(val) => (&self.is_tee_local, val),
            _ => bail_illegal_opcode!(instr),
        };
        selector.assign(region, offset, F::one());
        self.index
            .assign(region, offset, F::from(index.to_usize() as u64));
        Ok(())
    }
}
