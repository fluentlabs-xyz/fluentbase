use crate::rw_builder::{copy_row::CopyRow, rw_row::RwRow, RwBuilder};
use fluentbase_rwasm::{
    common::UntypedValue,
    engine::{bytecode::Instruction, Tracer, TracerInstrState},
};
use halo2_proofs::plonk;
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum GadgetError {
    MissingNext,
    OutOfStack,
    OutOfMemory,
    Plonk(plonk::Error),
}

pub const MAX_STACK_HEIGHT: usize = 1024;
pub const MAX_TABLE_SIZE: usize = 1024;

#[derive(Debug)]
pub struct ExecStep {
    pub(crate) trace: TracerInstrState,
    pub(crate) next_trace: Option<TracerInstrState>,
    pub(crate) global_memory: Vec<u8>,
    pub(crate) global_table: BTreeMap<u32, UntypedValue>,
    pub(crate) rw_rows: Vec<RwRow>,
    pub(crate) copy_rows: Vec<CopyRow>,
    pub(crate) rw_counter: usize,
}

impl ExecStep {
    pub fn instr(&self) -> &Instruction {
        &self.trace.opcode
    }

    pub fn stack_pointer(&self) -> u64 {
        MAX_STACK_HEIGHT as u64 - self.trace.stack.len() as u64
    }

    pub fn curr_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        Ok(self.trace.stack[self.trace.stack.len() - nth - 1])
    }

    pub fn curr_nth_stack_addr(&self, nth: usize) -> Result<u32, GadgetError> {
        Ok((MAX_STACK_HEIGHT - self.trace.stack.len() + nth) as u32)
    }

    pub fn next_nth_stack_value(&self, nth: usize) -> Result<UntypedValue, GadgetError> {
        self.next_trace
            .clone()
            .map(|trace| trace.stack[trace.stack.len() - nth - 1])
            .ok_or(GadgetError::MissingNext)
    }

    pub fn next_nth_stack_addr(&self, nth: usize) -> Result<u32, GadgetError> {
        self.next_trace
            .clone()
            .map(|trace| (MAX_STACK_HEIGHT - trace.stack.len() + nth) as u32)
            .ok_or(GadgetError::MissingNext)
    }

    pub fn read_memory<'a>(
        &self,
        offset: u64,
        dst: *mut u8,
        length: u32,
    ) -> Result<(), GadgetError> {
        let (sum, overflow) = offset.overflowing_add(length as u64);
        if overflow || sum > self.global_memory.len() as u64 {
            return Err(GadgetError::OutOfMemory);
        }
        unsafe {
            std::ptr::copy(
                self.global_memory.as_ptr().add(offset as usize),
                dst,
                length as usize,
            );
        }
        Ok(())
    }

    pub fn read_table_size(&self, table_idx: u32) -> usize {
        let size_addr = table_idx * 1024;
        let size = self
            .global_table
            .get(&size_addr)
            .copied()
            .unwrap_or_default();
        size.to_bits() as usize
    }

    pub fn read_table_elem(&self, table_idx: u32, elem_idx: u32) -> Option<UntypedValue> {
        let elem_addr = table_idx * (MAX_TABLE_SIZE as u32) + elem_idx + 1;
        self.global_table.get(&elem_addr).copied()
    }

    pub fn curr(&self) -> &TracerInstrState {
        &self.trace
    }

    pub fn next(&self) -> Result<&TracerInstrState, GadgetError> {
        self.next_trace.as_ref().ok_or(GadgetError::MissingNext)
    }
}

pub struct ExecSteps(pub Vec<ExecStep>);

impl ExecSteps {
    pub fn from_tracer(tracer: &Tracer) -> Result<Self, GadgetError> {
        let mut res = Self(Vec::new());

        let mut global_memory = Vec::new();
        let mut global_table = BTreeMap::<u32, UntypedValue>::new();
        let mut rw_counter = 0;

        for (i, trace) in tracer.logs.iter().cloned().enumerate() {
            for memory_change in trace.memory_changes.iter() {
                let max_offset = (memory_change.offset + memory_change.len) as usize;
                if max_offset > global_memory.len() {
                    global_memory.resize(max_offset, 0)
                }
                global_memory[(memory_change.offset as usize)..max_offset]
                    .copy_from_slice(memory_change.data.as_slice());
            }
            for table_change in trace.table_changes.iter() {
                let elem_addr = table_change.table_idx * 1024 + table_change.elem_idx + 1;
                global_table.insert(elem_addr, table_change.func_ref);
                let size_addr = table_change.table_idx * 1024;
                global_table.insert(size_addr, UntypedValue::from(0));
                let table_size = global_table
                    .keys()
                    .filter(|key| (*key / 1024) == table_change.table_idx)
                    .count();
                global_table.insert(size_addr, UntypedValue::from(table_size - 1));
            }
            let mut step = ExecStep {
                trace,
                next_trace: tracer.logs.get(i + 1).cloned(),
                global_memory: global_memory.clone(),
                global_table: global_table.clone(),
                rw_rows: vec![],
                copy_rows: vec![],
                rw_counter,
            };
            let mut rw_builder = RwBuilder::new();
            rw_builder.build(&mut step)?;
            rw_counter += step.rw_rows.len();
            res.0.push(step);
        }

        Ok(res)
    }

    pub fn get_rw_rows(&self) -> Vec<RwRow> {
        let mut res = Vec::new();
        for step in self.0.iter() {
            res.extend(&step.rw_rows);
        }
        res
    }
}
