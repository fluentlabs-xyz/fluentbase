use crate::{
    common::UntypedValue,
    engine::bytecode::{InstrMeta, Instruction, TableIdx},
    Extern,
};
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use core::{
    fmt::{Debug, Formatter},
    mem::take,
};

#[derive(Debug, Clone)]
pub struct TracerMemoryState {
    pub offset: u32,
    pub len: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TraceTableState {
    pub table_idx: u32,
    pub elem_idx: u32,
    pub func_ref: UntypedValue,
}

#[derive(Debug, Clone)]
pub struct TraceTableSizeState {
    pub table_idx: u32,
    pub init: u32,
    pub delta: u32,
}

#[derive(Debug, Clone)]
pub struct TracerInstrState {
    pub program_counter: u32,
    pub opcode: Instruction,
    pub memory_changes: Vec<TracerMemoryState>,
    pub table_changes: Vec<TraceTableState>,
    pub table_size_changes: Vec<TraceTableSizeState>,
    pub stack: Vec<UntypedValue>,
    pub next_table_idx: Option<TableIdx>,
    pub source_pc: u32,
    pub code: u16,
    pub memory_size: u32,
    pub index: usize,
    pub consumed_fuel: u64,
    pub call_id: u32,
}

#[derive(Default, Debug, Clone)]
pub struct TracerFunctionMeta {
    pub fn_index: u32,
    pub max_stack_height: u32,
    pub num_locals: u32,
    pub fn_name: String,
}

#[derive(Default, Clone)]
pub struct TracerGlobalVariable {
    pub index: u32,
    pub value: u64,
}

#[derive(Default, Clone)]
pub struct Tracer {
    pub global_memory: Vec<TracerMemoryState>,
    pub logs: Vec<TracerInstrState>,
    pub memory_changes: Vec<TracerMemoryState>,
    pub table_changes: Vec<TraceTableState>,
    pub table_size_changes: Vec<TraceTableSizeState>,
    pub fns_meta: Vec<TracerFunctionMeta>,
    pub global_variables: Vec<TracerGlobalVariable>,
    pub extern_names: BTreeMap<u32, String>,
    pub nested_calls: u32,
}

impl Debug for Tracer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "global_memory: {:?}; logs: {:?}; memory_changes: {:?}; fns_meta: {:?}",
            self.global_memory, self.logs, self.memory_changes, self.fns_meta
        )
    }
}

impl Tracer {
    pub fn merge_nested_call(&mut self, tracer: &Tracer) {
        self.nested_calls += 1;
        for mut log in tracer.logs.iter().cloned() {
            log.call_id = self.nested_calls;
            self.logs.push(log);
        }
    }

    pub fn global_memory(&mut self, offset: u32, len: u32, memory: &[u8]) {
        self.global_memory.push(TracerMemoryState {
            offset,
            len,
            data: Vec::from(memory),
        });
    }

    pub fn get_last_pc(&self) -> Option<u32> {
        self.logs.last().map(|opcode| opcode.source_pc)
    }

    pub fn register_extern(&mut self, ex: Extern, name: &Box<str>, entity_index: u32) {
        match ex {
            Extern::Global(_) => {}
            Extern::Table(_) => {}
            Extern::Memory(_) => {}
            Extern::Func(_) => {
                self.extern_names
                    .insert(entity_index, name.clone().into_string());
            }
        }
    }

    pub fn pre_opcode_state(
        &mut self,
        program_counter: u32,
        opcode: Instruction,
        stack: Vec<UntypedValue>,
        meta: &InstrMeta,
        memory_size: u32,
        consumed_fuel: u64,
    ) {
        let memory_changes = take(&mut self.memory_changes);
        let table_changes = take(&mut self.table_changes);
        let table_size_changes = take(&mut self.table_size_changes);
        let opcode_state = TracerInstrState {
            program_counter,
            opcode,
            memory_changes,
            table_changes,
            table_size_changes,
            stack,
            next_table_idx: None,
            source_pc: meta.offset() as u32,
            code: meta.opcode(),
            memory_size,
            index: meta.index(),
            consumed_fuel,
            call_id: 0,
        };
        self.logs.push(opcode_state.clone());
    }

    pub fn remember_next_table(&mut self, table_idx: TableIdx) {
        self.logs.last_mut().map(|v| {
            v.next_table_idx = Some(table_idx);
        });
    }

    pub fn function_call(
        &mut self,
        fn_index: u32,
        max_stack_height: usize,
        num_locals: usize,
        fn_name: String,
    ) {
        let resolved_name = self.extern_names.get(&fn_index).unwrap_or(&fn_name);
        self.fns_meta.push(TracerFunctionMeta {
            fn_index,
            max_stack_height: max_stack_height as u32,
            num_locals: num_locals as u32,
            fn_name: resolved_name.clone(),
        })
    }

    pub fn global_variable(&mut self, value: UntypedValue, index: u32) {
        self.global_variables.push(TracerGlobalVariable {
            value: value.to_bits(),
            index,
        })
    }

    pub fn memory_change(&mut self, offset: u32, len: u32, memory: &[u8]) {
        self.memory_changes.push(TracerMemoryState {
            offset,
            len,
            data: Vec::from(memory),
        });
    }

    pub fn table_change(&mut self, table_idx: u32, elem_idx: u32, func_ref: UntypedValue) {
        self.table_changes.push(TraceTableState {
            table_idx,
            elem_idx,
            func_ref,
        });
    }

    pub fn table_size_change(&mut self, table_idx: u32, init: u32, delta: u32) {
        self.table_size_changes.push(TraceTableSizeState {
            table_idx,
            init,
            delta,
        });
    }
}
