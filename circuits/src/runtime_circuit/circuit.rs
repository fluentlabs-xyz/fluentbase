use crate::{
    exec_step::{ExecSteps, GadgetError},
    lookup_table::{
        BitwiseCheckLookup,
        CopyLookup,
        FixedLookup,
        PublicInputLookup,
        RangeCheckLookup,
        RwLookup,
        RwasmLookup,
    },
    runtime_circuit::{
        execution_gadget::ExecutionContextGadget,
        execution_state::ExecutionState,
        opcodes::{
            op_bin::OpBinGadget,
            op_bitwise::OpBitwiseGadget,
            op_break::OpBreakGadget,
            op_call::OpCallGadget,
            op_const::OpConstGadget,
            op_consume_fuel::OpConsumeFuel,
            op_conversion::OpConversionGadget,
            op_drop::OpDropGadget,
            op_extend::OpExtendGadget,
            op_global::OpGlobalGadget,
            op_load::OpLoadGadget,
            op_local::OpLocalGadget,
            op_memory_copy::OpMemoryCopyGadget,
            op_memory_fill::OpMemoryFillGadget,
            op_memory_grow::OpMemoryGrowGadget,
            op_memory_init::OpMemoryInitGadget,
            op_memory_size::OpMemorySizeGadget,
            op_reffunc::OpRefFuncGadget,
            op_return::OpReturnGadget,
            op_select::OpSelectGadget,
            op_shift::OpShiftGadget,
            op_store::OpStoreGadget,
            op_test::OpTestGadget,
            op_unary::OpUnaryGadget,
            op_unreachable::OpUnreachableGadget,
            table_ops::{
                copy::OpTableCopyGadget,
                fill::OpTableFillGadget,
                get::OpTableGetGadget,
                grow::OpTableGrowGadget,
                set::OpTableSetGadget,
                size::OpTableSizeGadget,
            },
            ExecStep,
        },
        platform::{sys_halt::SysHaltGadget, sys_read::SysReadGadget, sys_write::SysWriteGadget},
        responsible_opcode::ResponsibleOpcodeTable,
    },
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};

#[derive(Clone)]
pub struct RuntimeCircuitConfig<F: Field> {
    // wasm opcodes
    unreachable_gadget: ExecutionContextGadget<F, OpUnreachableGadget<F>>,
    consume_fuel_gadget: ExecutionContextGadget<F, OpConsumeFuel<F>>,
    bin_gadget: ExecutionContextGadget<F, OpBinGadget<F>>,
    break_gadget: ExecutionContextGadget<F, OpBreakGadget<F>>,
    call_gadget: ExecutionContextGadget<F, OpCallGadget<F>>,
    const_gadget: ExecutionContextGadget<F, OpConstGadget<F>>,
    reffunc_gadget: ExecutionContextGadget<F, OpRefFuncGadget<F>>,
    conversion_gadget: ExecutionContextGadget<F, OpConversionGadget<F>>,
    drop_gadget: ExecutionContextGadget<F, OpDropGadget<F>>,
    global_gadget: ExecutionContextGadget<F, OpGlobalGadget<F>>,
    local_gadget: ExecutionContextGadget<F, OpLocalGadget<F>>,
    select_gadget: ExecutionContextGadget<F, OpSelectGadget<F>>,
    unary_gadget: ExecutionContextGadget<F, OpUnaryGadget<F>>,
    test_gadget: ExecutionContextGadget<F, OpTestGadget<F>>,
    store_gadget: ExecutionContextGadget<F, OpStoreGadget<F>>,
    load_gadget: ExecutionContextGadget<F, OpLoadGadget<F>>,
    table_copy_gadget: ExecutionContextGadget<F, OpTableCopyGadget<F>>,
    table_fill_gadget: ExecutionContextGadget<F, OpTableFillGadget<F>>,
    table_get_gadget: ExecutionContextGadget<F, OpTableGetGadget<F>>,
    table_grow_gadget: ExecutionContextGadget<F, OpTableGrowGadget<F>>,
    table_set_gadget: ExecutionContextGadget<F, OpTableSetGadget<F>>,
    table_size_gadget: ExecutionContextGadget<F, OpTableSizeGadget<F>>,
    bitwise_gadget: ExecutionContextGadget<F, OpBitwiseGadget<F>>,
    extend_gadget: ExecutionContextGadget<F, OpExtendGadget<F>>,
    shift_gadget: ExecutionContextGadget<F, OpShiftGadget<F>>,
    memory_copy_gadget: ExecutionContextGadget<F, OpMemoryCopyGadget<F>>,
    memory_grow_gadget: ExecutionContextGadget<F, OpMemoryGrowGadget<F>>,
    memory_size_gadget: ExecutionContextGadget<F, OpMemorySizeGadget<F>>,
    memory_fill_gadget: ExecutionContextGadget<F, OpMemoryFillGadget<F>>,
    memory_init_gadget: ExecutionContextGadget<F, OpMemoryInitGadget<F>>,
    return_gadget: ExecutionContextGadget<F, OpReturnGadget<F>>,
    // system calls TODO: "lets design an extension library for this"
    sys_halt_gadget: ExecutionContextGadget<F, SysHaltGadget<F>>,
    sys_read_gadget: ExecutionContextGadget<F, SysReadGadget<F>>,
    sys_write_gadget: ExecutionContextGadget<F, SysWriteGadget<F>>,
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
        copy_lookup: &impl CopyLookup<F>,
        bitwise_check_lookup: &impl BitwiseCheckLookup<F>,
    ) -> Self {
        let responsible_opcode_table = ResponsibleOpcodeTable::configure(cs);

        macro_rules! configure_gadget {
            () => {
                ExecutionContextGadget::configure(
                    cs,
                    rwasm_lookup,
                    state_lookup,
                    &responsible_opcode_table,
                    range_check_lookup,
                    fixed_lookup,
                    public_input_lookup,
                    copy_lookup,
                    bitwise_check_lookup,
                )
            };
        }

        Self {
            // wasm opcodes
            unreachable_gadget: configure_gadget!(),
            consume_fuel_gadget: configure_gadget!(),
            bin_gadget: configure_gadget!(),
            break_gadget: configure_gadget!(),
            call_gadget: configure_gadget!(),
            const_gadget: configure_gadget!(),
            reffunc_gadget: configure_gadget!(),
            conversion_gadget: configure_gadget!(),
            drop_gadget: configure_gadget!(),
            global_gadget: configure_gadget!(),
            local_gadget: configure_gadget!(),
            select_gadget: configure_gadget!(),
            unary_gadget: configure_gadget!(),
            test_gadget: configure_gadget!(),
            store_gadget: configure_gadget!(),
            load_gadget: configure_gadget!(),
            table_copy_gadget: configure_gadget!(),
            table_fill_gadget: configure_gadget!(),
            table_get_gadget: configure_gadget!(),
            table_grow_gadget: configure_gadget!(),
            table_set_gadget: configure_gadget!(),
            table_size_gadget: configure_gadget!(),
            bitwise_gadget: configure_gadget!(),
            extend_gadget: configure_gadget!(),
            shift_gadget: configure_gadget!(),
            memory_copy_gadget: configure_gadget!(),
            memory_grow_gadget: configure_gadget!(),
            memory_size_gadget: configure_gadget!(),
            memory_fill_gadget: configure_gadget!(),
            memory_init_gadget: configure_gadget!(),
            return_gadget: configure_gadget!(),
            // system calls
            sys_halt_gadget: configure_gadget!(),
            sys_read_gadget: configure_gadget!(),
            sys_write_gadget: configure_gadget!(),
            // other
            responsible_opcode_table,
        }
    }

    fn assign_sys_call(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        step: &ExecStep,
        rw_counter: usize,
        system_call: SysFuncIdx,
    ) -> Result<(), GadgetError> {
        match system_call {
            SysFuncIdx::IMPORT_SYS_HALT => self
                .sys_halt_gadget
                .assign(region, offset, step, rw_counter)?,
            SysFuncIdx::IMPORT_SYS_READ => self
                .sys_read_gadget
                .assign(region, offset, step, rw_counter)?,
            SysFuncIdx::IMPORT_SYS_WRITE => self
                .sys_write_gadget
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
        step: &ExecStep,
        rw_counter: usize,
    ) -> Result<(), Error> {
        let execution_state = ExecutionState::from_opcode(*step.instr());
        let res = match execution_state {
            ExecutionState::WASM_UNREACHABLE => self
                .unreachable_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_CONSUME_FUEL => self
                .consume_fuel_gadget
                .assign(region, offset, step, rw_counter),
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
            ExecutionState::WASM_REFFUNC => {
                self.reffunc_gadget.assign(region, offset, step, rw_counter)
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

            ExecutionState::WASM_TABLE_COPY => self
                .table_copy_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TABLE_FILL => self
                .table_fill_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TABLE_GET => self
                .table_get_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TABLE_GROW => self
                .table_grow_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TABLE_SET => self
                .table_set_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TABLE_SIZE => self
                .table_size_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_BITWISE => {
                self.bitwise_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_EXTEND => {
                self.extend_gadget.assign(region, offset, step, rw_counter)
            }

            ExecutionState::WASM_SHIFT => {
                self.shift_gadget.assign(region, offset, step, rw_counter)
            }

            ExecutionState::WASM_MEMORY_COPY => self
                .memory_copy_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_MEMORY_GROW => self
                .memory_grow_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_MEMORY_SIZE => self
                .memory_size_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_MEMORY_FILL => self
                .memory_fill_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_MEMORY_INIT => self
                .memory_init_gadget
                .assign(region, offset, step, rw_counter),
            ExecutionState::WASM_TEST => self.test_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_STORE => {
                self.store_gadget.assign(region, offset, step, rw_counter)
            }
            ExecutionState::WASM_LOAD => self.load_gadget.assign(region, offset, step, rw_counter),
            ExecutionState::WASM_RETURN => {
                self.return_gadget.assign(region, offset, step, rw_counter)
            }
            _ => unreachable!("not supported gadget {:?}", execution_state),
        };
        // TODO: "do normal error handling here"
        res.unwrap();
        Ok(())
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        exec_steps: &ExecSteps,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "runtime opcodes",
            |mut region| {
                for (i, trace) in exec_steps.0.iter().enumerate() {
                    self.assign_trace_step(&mut region, i, trace, trace.rw_counter)?;
                }
                Ok(())
            },
        )?;
        self.responsible_opcode_table.load(layouter)?;
        Ok(())
    }
}
