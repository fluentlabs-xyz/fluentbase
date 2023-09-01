use crate::{
    lookup_table::{RangeCheckLookup, RwLookup, RwasmLookup},
    runtime_circuit::{
        execution_gadget::ExecutionGadgetRow,
        execution_state::ExecutionState,
        opcodes::{
            op_bin::BinGadget,
            op_const::ConstGadget,
            op_drop::DropGadget,
            op_local::LocalGadget,
            TraceStep,
        },
        responsible_opcode::ResponsibleOpcodeTable,
    },
    util::Field,
};
use fluentbase_rwasm::engine::Tracer;
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};

#[derive(Clone)]
pub struct RuntimeCircuitConfig<F: Field> {
    bin_gadget: ExecutionGadgetRow<F, BinGadget<F>>,
    const_gadget: ExecutionGadgetRow<F, ConstGadget<F>>,
    drop_gadget: ExecutionGadgetRow<F, DropGadget<F>>,
    local_gadget: ExecutionGadgetRow<F, LocalGadget<F>>,
    // runtime state gadgets
    responsible_opcode_table: ResponsibleOpcodeTable<F>,
}

impl<F: Field> RuntimeCircuitConfig<F> {
    #[allow(unused_variables)]
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        rwasm_lookup: &impl RwasmLookup<F>,
        state_lookup: &impl RwLookup<F>,
        range_check_lookup: &impl RangeCheckLookup<F>,
    ) -> Self {
        let responsible_opcode_table = ResponsibleOpcodeTable::configure(cs);
        macro_rules! configure_gadget {
            () => {
                ExecutionGadgetRow::configure(
                    cs,
                    rwasm_lookup,
                    state_lookup,
                    &responsible_opcode_table,
                    range_check_lookup,
                )
            };
        }
        Self {
            bin_gadget: configure_gadget!(),
            const_gadget: configure_gadget!(),
            drop_gadget: configure_gadget!(),
            local_gadget: configure_gadget!(),
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
        let execution_state = ExecutionState::from_opcode(*step.instr());
        let res = match execution_state {
            ExecutionState::WASM_BIN => self.bin_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_CONST => {
                self.const_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_DROP => self.drop_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_LOCAL => {
                self.local_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_BREAK => {
                // do nothing for WASM_BREAK for now
                Ok(())
            }
            _ => unreachable!("not supported gadget {:?}", execution_state),
        };
        // TODO: "do normal error handling here"
        res.unwrap();
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
