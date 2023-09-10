use crate::{
    constraint_builder::{AdviceColumn, FixedColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
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

#[derive(Clone)]
pub struct OpBreakGadget<F: Field> {
    is_br: FixedColumn,
    is_br_if_eqz: FixedColumn,
    is_br_if_nez: FixedColumn,
    is_br_adjust: FixedColumn,
    is_br_adjust_if_nez: FixedColumn,
    value: AdviceColumn,
    value_inv: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpBreakGadget<F> {
    const NAME: &'static str = "WASM_BREAK";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_BREAK;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_br = cb.query_fixed();
        let is_br_if_eqz = cb.query_fixed();
        let is_br_if_nez = cb.query_fixed();
        let is_br_adjust = cb.query_fixed();
        let is_br_adjust_if_nez = cb.query_fixed();
        let value = cb.query_cell();
        let value_inv = cb.query_cell();

        cb.require_exactly_one_selector([
            is_br.current(),
            is_br_if_eqz.current(),
            is_br_if_nez.current(),
            is_br_adjust.current(),
            is_br_adjust_if_nez.current(),
        ]);

        cb.if_rwasm_opcode(is_br.current(), Instruction::Br(Default::default()), |cb| {
            cb.next_pc_jump(cb.query_rwasm_value())
        });
        cb.if_rwasm_opcode(
            is_br_if_eqz.current(),
            Instruction::BrIfEqz(Default::default()),
            |cb| {
                cb.stack_pop(value.current());
                let is_zero = 1.expr() - value.current() * value_inv.current();
                cb.condition(is_zero, |cb| cb.next_pc_jump(cb.query_rwasm_value()));
            },
        );
        cb.if_rwasm_opcode(
            is_br_if_nez.current(),
            Instruction::BrIfNez(Default::default()),
            |cb| {
                cb.stack_pop(value.current());
                let is_not_zero = value.current() * value_inv.current();
                cb.condition(is_not_zero, |cb| cb.next_pc_jump(cb.query_rwasm_value()));
            },
        );
        cb.if_rwasm_opcode(
            is_br_adjust.current(),
            Instruction::BrAdjust(Default::default()),
            |cb| cb.next_pc_jump(cb.query_rwasm_value()),
        );
        cb.if_rwasm_opcode(
            is_br_adjust_if_nez.current(),
            Instruction::BrAdjustIfNez(Default::default()),
            |cb| {
                cb.stack_pop(value.current());
                let is_not_zero = value.current() * value_inv.current();
                cb.condition(is_not_zero, |cb| cb.next_pc_jump(cb.query_rwasm_value()));
            },
        );

        Self {
            is_br,
            is_br_if_eqz,
            is_br_if_nez,
            is_br_adjust,
            is_br_adjust_if_nez,
            value,
            value_inv,
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
            Instruction::Br(_) => {
                self.is_br.assign(region, offset, 1u64);
            }
            Instruction::BrIfEqz(_) => {
                self.is_br_if_eqz.assign(region, offset, 1u64);
            }
            Instruction::BrIfNez(_) => {
                self.is_br_if_nez.assign(region, offset, 1u64);
            }
            Instruction::BrAdjust(_) => {}
            Instruction::BrAdjustIfNez(_) => {}
            _ => unreachable!("illegal opcode place {:?}", trace.instr()),
        }
        todo!()
    }
}
