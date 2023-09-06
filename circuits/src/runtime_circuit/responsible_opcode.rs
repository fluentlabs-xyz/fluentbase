use crate::{
    constraint_builder::{FixedColumn, Query},
    lookup_table::{ResponsibleOpcodeLookup, N_RESPONSIBLE_OPCODE_LOOKUP_TABLE},
    runtime_circuit::execution_state::ExecutionState,
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;
use strum::IntoEnumIterator;

#[derive(Clone)]
pub struct ResponsibleOpcodeTable<F: Field> {
    execution_state: FixedColumn,
    opcode: FixedColumn,
    affects_pc: FixedColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ResponsibleOpcodeTable<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            execution_state: FixedColumn(cs.fixed_column()),
            opcode: FixedColumn(cs.fixed_column()),
            affects_pc: FixedColumn(cs.fixed_column()),
            marker: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "responsible opcode table",
            |mut region| {
                let mut offset = 0;
                for state in ExecutionState::iter() {
                    let responsible_opcodes = state.responsible_opcodes();
                    for opcode in responsible_opcodes {
                        self.execution_state
                            .assign(&mut region, offset, state.to_u64());
                        self.opcode
                            .assign(&mut region, offset, opcode.code_value() as u64);
                        self.affects_pc
                            .assign(&mut region, offset, opcode.affects_pc() as u64);
                        offset += 1;
                    }
                }
                for sys_fn in SysFuncIdx::iter() {
                    let state = ExecutionState::WASM_CALL_HOST(sys_fn);
                    self.execution_state
                        .assign(&mut region, offset, state.to_u64());
                    self.opcode.assign(
                        &mut region,
                        offset,
                        Instruction::Call(Default::default()).code_value() as u64,
                    );
                    self.affects_pc.assign(&mut region, offset, 1u64);
                    offset += 1;
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: Field> ResponsibleOpcodeLookup<F> for ResponsibleOpcodeTable<F> {
    fn lookup_responsible_opcode_table(&self) -> [Query<F>; N_RESPONSIBLE_OPCODE_LOOKUP_TABLE] {
        [
            self.execution_state.current(),
            self.opcode.current(),
            self.affects_pc.current(),
        ]
    }
}
