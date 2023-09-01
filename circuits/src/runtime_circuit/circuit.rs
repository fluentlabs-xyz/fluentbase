use crate::{
    constraint_builder::{AdviceColumn, SelectorColumn},
    lookup_table::{ResponsibleOpcodeLookup, RwLookup, RwasmLookup},
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, StateTransition},
        opcodes::{
            op_const::ConstGadget,
            op_drop::DropGadget,
            op_local::LocalGadget,
            ExecutionGadget,
            GadgetError,
            TraceStep,
        },
        responsible_opcode::ResponsibleOpcodeTable,
    },
    util::Field,
};
use fluentbase_rwasm::engine::{bytecode::Instruction, Tracer};
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};
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
    ) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let mut state_transition = StateTransition::configure(cs);
        let mut cb = OpConstraintBuilder::new(cs, q_enable, &mut state_transition);
        let [index, opcode, value] = cb.query_rwasm_table();
        cb.rwasm_lookup(index.current(), opcode.current(), value.current());
        cb.execution_state_lookup(G::EXECUTION_STATE, opcode.current());
        let gadget_config = G::configure(&mut cb);
        cb.build(rwasm_lookup, state_lookup, responsible_opcode_lookup);
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

#[derive(Clone)]
pub struct RuntimeCircuitConfig<F: Field> {
    const_gadget: ExecutionGadgetRow<F, ConstGadget<F>>,
    drop_gadget: ExecutionGadgetRow<F, DropGadget<F>>,
    local_gadget: ExecutionGadgetRow<F, LocalGadget<F>>,
    // runtime state gadgets
    responsible_opcode_table: ResponsibleOpcodeTable<F>,
}

impl<F: Field> RuntimeCircuitConfig<F> {
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        rwasm_lookup: &impl RwasmLookup<F>,
        state_lookup: &impl RwLookup<F>,
    ) -> Self {
        let responsible_opcode_table = ResponsibleOpcodeTable::configure(cs);
        Self {
            const_gadget: ExecutionGadgetRow::configure(
                cs,
                rwasm_lookup,
                state_lookup,
                &responsible_opcode_table,
            ),
            drop_gadget: ExecutionGadgetRow::configure(
                cs,
                rwasm_lookup,
                state_lookup,
                &responsible_opcode_table,
            ),
            local_gadget: ExecutionGadgetRow::configure(
                cs,
                rwasm_lookup,
                state_lookup,
                &responsible_opcode_table,
            ),
            responsible_opcode_table,
        }
    }

    #[allow(unused_variables)]
    fn assign_trace_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &TraceStep,
        rw_counter: usize,
    ) -> Result<(), Error> {
        let res = match step.instr() {
            Instruction::I32Const(_) | Instruction::I64Const(_) => {
                self.const_gadget.assign(region, offset, step, rw_counter)
            }
            Instruction::Drop => self.drop_gadget.assign(region, offset, step, rw_counter),
            Instruction::LocalGet(_) | Instruction::LocalSet(_) | Instruction::LocalTee(_) => {
                self.local_gadget.assign(region, offset, step, rw_counter)
            }
            Instruction::Return(_) => {
                // just skip for now
                Ok(())
            }
            _ => unreachable!("not supported opcode {:?}", step.instr()),
        };
        Ok(())
    }

    pub fn assign(&self, layouter: &mut impl Layouter<F>, tracer: &Tracer) -> Result<(), Error> {
        layouter.assign_region(
            || "runtime opcodes",
            |mut region| {
                let mut rw_counter = 0;
                for (i, trace) in tracer.logs.iter().cloned().enumerate() {
                    let step = TraceStep::new(trace, tracer.logs.get(i + 1).cloned());
                    self.assign_trace_step(&mut region, i, &step, rw_counter)?;
                    rw_counter += step.instr().get_rw_ops().len();
                }
                Ok(())
            },
        )?;
        self.responsible_opcode_table.load(layouter)?;
        Ok(())
    }
}
