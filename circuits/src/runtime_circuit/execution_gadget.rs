use crate::{
    constraint_builder::{dynamic_cell_manager::DynamicCellManager, AdviceColumn, SelectorColumn},
    lookup_table::{
        BitwiseCheckLookup,
        CopyLookup,
        FixedLookup,
        PublicInputLookup,
        RangeCheckLookup,
        ResponsibleOpcodeLookup,
        RwLookup,
        RwasmLookup,
    },
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, StateTransition},
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    util::Field,
};
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use log::debug;

#[derive(Clone)]
pub struct ExecutionContextGadget<F: Field, G: ExecutionGadget<F>> {
    gadget: G,
    q_enable: SelectorColumn,
    pc: AdviceColumn,
    opcode: AdviceColumn,
    value: AdviceColumn,
    state_transition: StateTransition<F>,
}

impl<F: Field, G: ExecutionGadget<F>> ExecutionContextGadget<F, G> {
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        dcm: &mut DynamicCellManager<F>,
        rwasm_lookup: &impl RwasmLookup<F>,
        state_lookup: &impl RwLookup<F>,
        responsible_opcode_lookup: &impl ResponsibleOpcodeLookup<F>,
        range_check_lookup: &impl RangeCheckLookup<F>,
        fixed_lookup: &impl FixedLookup<F>,
        public_input_lookup: &impl PublicInputLookup<F>,
        copy_lookup: &impl CopyLookup<F>,
        bitwise_check_lookup: &impl BitwiseCheckLookup<F>,
    ) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        // we store register states in state transition gadget
        let mut state_transition = StateTransition::configure(cs);
        let mut cb = OpConstraintBuilder::new(cs, dcm, q_enable, &mut state_transition);
        // extract rwasm table with opcode and value fields (for lookup)
        let [pc, opcode, value] = cb.rwasm_table();
        // make sure opcode and value fields are correct and set properly
        cb.rwasm_lookup(pc.current(), opcode.current(), value.current());
        cb.execution_state_lookup(G::EXECUTION_STATE, cb.query_rwasm_opcode());
        debug!(
            "ExecutionGadget::configure ExecutionGadget::NAME={}",
            G::NAME
        );
        // configure gadget and build gates
        G::configure_state_transition(&mut cb);
        let gadget_config = G::configure(&mut cb);
        cb.build(
            rwasm_lookup,
            state_lookup,
            responsible_opcode_lookup,
            range_check_lookup,
            fixed_lookup,
            public_input_lookup,
            copy_lookup,
            bitwise_check_lookup,
        );
        ExecutionContextGadget {
            gadget: gadget_config,
            pc,
            opcode,
            value,
            q_enable,
            state_transition,
        }
    }

    pub fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &ExecStep,
        rw_counter: usize,
    ) -> Result<(), GadgetError> {
        self.q_enable.enable(region, offset);
        // assign rwasm params (code, value)
        let pc = step.curr().source_pc as u64;
        self.pc.assign(region, offset, F::from(pc));
        let opcode = step.curr().code as u64;
        self.opcode.assign(region, offset, F::from(opcode));
        let value = step.curr().opcode.aux_value().unwrap_or_default();
        self.value.assign(region, offset, F::from(value.to_bits()));
        // assign state transition
        self.state_transition.assign(
            region,
            offset,
            step.stack_pointer(),
            rw_counter as u64,
            step.call_id as u64,
        );
        // assign opcode gadget
        self.gadget.assign_exec_step(region, offset, step)
    }
}
