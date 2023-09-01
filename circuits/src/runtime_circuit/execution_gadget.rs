use crate::{
    constraint_builder::{AdviceColumn, SelectorColumn},
    lookup_table::{RangeCheckLookup, ResponsibleOpcodeLookup, RwLookup, RwasmLookup},
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, StateTransition},
        opcodes::{ExecutionGadget, GadgetError, TraceStep},
    },
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct ExecutionGadgetRow<F: Field, G: ExecutionGadget<F>> {
    gadget: G,
    q_enable: SelectorColumn,
    index: AdviceColumn,
    code: AdviceColumn,
    value: AdviceColumn,
    state_transition: StateTransition<F>,
    pd: PhantomData<F>,
}

impl<F: Field, G: ExecutionGadget<F>> ExecutionGadgetRow<F, G> {
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        rwasm_lookup: &impl RwasmLookup<F>,
        state_lookup: &impl RwLookup<F>,
        responsible_opcode_lookup: &impl ResponsibleOpcodeLookup<F>,
        range_check_lookup: &impl RangeCheckLookup<F>,
    ) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let mut state_transition = StateTransition::configure(cs);
        let mut cb = OpConstraintBuilder::new(cs, q_enable, &mut state_transition);
        let [index, opcode, value] = cb.query_rwasm_table();
        cb.rwasm_lookup(index.current(), opcode.current(), value.current());
        cb.execution_state_lookup(G::EXECUTION_STATE, opcode.current());
        let gadget_config = G::configure(&mut cb);
        cb.build(
            rwasm_lookup,
            state_lookup,
            responsible_opcode_lookup,
            range_check_lookup,
        );
        ExecutionGadgetRow {
            gadget: gadget_config,
            index,
            code: opcode,
            value,
            q_enable,
            state_transition,
            pd: Default::default(),
        }
    }

    pub fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &TraceStep,
        rw_counter: usize,
    ) -> Result<(), GadgetError> {
        self.q_enable.enable(region, offset);
        // assign rwasm params (index, code, value)
        self.index
            .assign(region, offset, F::from(step.curr().source_pc as u64));
        self.code
            .assign(region, offset, F::from(step.curr().code as u64));
        let value = step.curr().opcode.aux_value().unwrap_or_default();
        self.value.assign(region, offset, F::from(value.to_bits()));
        // assign state transition
        self.state_transition
            .assign(region, offset, step.stack_pointer(), rw_counter as u64);
        // assign opcode gadget
        self.gadget.assign_exec_step(region, offset, step)
    }
}
