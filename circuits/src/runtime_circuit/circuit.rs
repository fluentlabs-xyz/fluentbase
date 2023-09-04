use crate::{
    lookup_table::{FixedLookup, RangeCheckLookup, RwLookup, RwasmLookup},
    runtime_circuit::{
        execution_gadget::ExecutionGadgetRow,
        execution_state::ExecutionState,
        opcodes::{
            op_bin::OpBinGadget,
            op_const::OpConstGadget,
            op_conversion::OpConversionGadget,
            op_drop::OpDropGadget,
            op_global::OpGlobalGadget,
            op_local::OpLocalGadget,
            op_select::OpSelectGadget,
            op_unary::OpUnaryGadget,
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
    bin_gadget: ExecutionGadgetRow<F, OpBinGadget<F>>,
    const_gadget: ExecutionGadgetRow<F, OpConstGadget<F>>,
    conversion_gadget: ExecutionGadgetRow<F, OpConversionGadget<F>>,
    drop_gadget: ExecutionGadgetRow<F, OpDropGadget<F>>,
    global_gadget: ExecutionGadgetRow<F, OpGlobalGadget<F>>,
    local_gadget: ExecutionGadgetRow<F, OpLocalGadget<F>>,
    select_gadget: ExecutionGadgetRow<F, OpSelectGadget<F>>,
    unary_gadget: ExecutionGadgetRow<F, OpUnaryGadget<F>>,
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
        fixed_lookup: &impl FixedLookup<F>,
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
                    fixed_lookup,
                )
            };
        }
        Self {
            bin_gadget: configure_gadget!(),
            const_gadget: configure_gadget!(),
            conversion_gadget: configure_gadget!(),
            drop_gadget: configure_gadget!(),
            global_gadget: configure_gadget!(),
            local_gadget: configure_gadget!(),
            select_gadget: configure_gadget!(),
            unary_gadget: configure_gadget!(),
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
            ExecutionState::WASM_CONVERSION => self
                .conversion_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_DROP => self.drop_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_GLOBAL => {
                self.global_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_LOCAL => {
                self.local_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_SELECT => {
                self.select_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_UNARY => {
                self.unary_gadget.assign(region, offset, step, rw_counter)
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
