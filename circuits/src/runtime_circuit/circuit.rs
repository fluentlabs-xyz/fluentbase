use crate::{
    lookup_table::{FixedLookup, PublicInputLookup, RangeCheckLookup, RwLookup, RwasmLookup},
    runtime_circuit::{
        execution_gadget::ExecutionGadgetRow,
        execution_state::ExecutionState,
        opcodes::{
            op_bin::OpBinGadget,
            op_break::OpBreakGadget,
            op_call::OpCallGadget,
            op_const::OpConstGadget,
            op_conversion::OpConversionGadget,
            op_drop::OpDropGadget,
            op_global::OpGlobalGadget,
            op_load::OpLoadGadget,
            op_local::OpLocalGadget,
            op_select::OpSelectGadget,
            op_store::OpStoreGadget,
            op_test::OpTestGadget,
            op_unary::OpUnaryGadget,
            TraceStep,
        },
        platform::sys_halt::SysHaltGadget,
        responsible_opcode::ResponsibleOpcodeTable,
    },
    trace_step::GadgetError,
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::engine::Tracer;
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};

#[derive(Clone)]
pub struct RuntimeCircuitConfig<F: Field> {
    // wasm opcodes
    bin_gadget: ExecutionGadgetRow<F, OpBinGadget<F>>,
    break_gadget: ExecutionGadgetRow<F, OpBreakGadget<F>>,
    call_gadget: ExecutionGadgetRow<F, OpCallGadget<F>>,
    const_gadget: ExecutionGadgetRow<F, OpConstGadget<F>>,
    conversion_gadget: ExecutionGadgetRow<F, OpConversionGadget<F>>,
    drop_gadget: ExecutionGadgetRow<F, OpDropGadget<F>>,
    global_gadget: ExecutionGadgetRow<F, OpGlobalGadget<F>>,
    local_gadget: ExecutionGadgetRow<F, OpLocalGadget<F>>,
    select_gadget: ExecutionGadgetRow<F, OpSelectGadget<F>>,
    unary_gadget: ExecutionGadgetRow<F, OpUnaryGadget<F>>,
    test_gadget: ExecutionGadgetRow<F, OpTestGadget<F>>,
    store_gadget: ExecutionGadgetRow<F, OpStoreGadget<F>>,
    load_gadget: ExecutionGadgetRow<F, OpLoadGadget<F>>,
    // system calls TODO: "lets design an extension library for this"
    sys_halt_gadget: ExecutionGadgetRow<F, SysHaltGadget<F>>,
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
        public_input_lookup: &impl PublicInputLookup<F>,
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
                    public_input_lookup,
                )
            };
        }
        Self {
            // wasm opcodes
            bin_gadget: configure_gadget!(),
            break_gadget: configure_gadget!(),
            call_gadget: configure_gadget!(),
            const_gadget: configure_gadget!(),
            conversion_gadget: configure_gadget!(),
            drop_gadget: configure_gadget!(),
            global_gadget: configure_gadget!(),
            local_gadget: configure_gadget!(),
            select_gadget: configure_gadget!(),
            unary_gadget: configure_gadget!(),
            test_gadget: configure_gadget!(),
            store_gadget: configure_gadget!(),
            load_gadget: configure_gadget!(),
            // system calls
            sys_halt_gadget: configure_gadget!(),
            responsible_opcode_table,
        }
    }

    fn assign_sys_call(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &TraceStep,
        rw_counter: usize,
        system_call: SysFuncIdx,
    ) -> Result<(), GadgetError> {
        match system_call {
            SysFuncIdx::IMPORT_SYS_HALT => self
                .sys_halt_gadget
                .assign(region, offset, step, rw_counter)?,
            _ => unreachable!("not supported sys call: {:?}", system_call),
        }
        Ok(())
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
            ExecutionState::WASM_BREAK => {
                self.break_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_CALL => self.call_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_CALL_HOST(system_call) => {
                self.assign_sys_call(region, offset, step, rw_counter, system_call)
            }
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
            ExecutionState::WASM_TEST => self.test_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_STORE => {
                self.store_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_LOAD => self.load_gadget.assign(region, offset, step, rw_counter),
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
                let mut global_memory = Vec::new();
                for (i, trace) in tracer.logs.iter().cloned().enumerate() {
                    for memory_change in trace.memory_changes.iter() {
                        let max_offset = (memory_change.offset + memory_change.len) as usize;
                        if max_offset > global_memory.len() {
                            global_memory.resize(max_offset, 0)
                        }
                        global_memory[(memory_change.offset as usize)..max_offset]
                            .copy_from_slice(memory_change.data.as_slice());
                    }
                    let step = TraceStep::new(
                        trace,
                        tracer.logs.get(i + 1).cloned(),
                        global_memory.clone(),
                    );
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
