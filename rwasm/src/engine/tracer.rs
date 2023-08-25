use crate::{
    common::UntypedValue,
    engine::bytecode::{InstrMeta, Instruction},
    Extern,
};
use core::fmt::{Debug, Formatter};
use std::{collections::BTreeMap, mem::take};

#[derive(Debug, Clone)]
pub struct MemoryState {
    pub offset: u32,
    pub len: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct OpCodeState {
    pub program_counter: u32,
    pub opcode: Instruction,
    pub memory_changes: Vec<MemoryState>,
    pub stack: Vec<u64>,
    pub source_pc: u32,
    pub code: u16,
}

#[derive(Debug)]
pub struct FunctionMeta {
    pub fn_index: u32,
    pub max_stack_height: u32,
    pub num_locals: u32,
    pub fn_name: String,
}

#[derive(Debug)]
pub struct GlobalVariable {
    pub index: u32,
    pub value: u64,
}

#[derive(Default)]
pub struct Tracer {
    global_memory: Vec<MemoryState>,
    logs: Vec<OpCodeState>,
    memory_changes: Vec<MemoryState>,
    fns_meta: Vec<FunctionMeta>,
    global_variables: Vec<GlobalVariable>,
    extern_names: BTreeMap<u32, String>,
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
    pub fn global_memory(&mut self, offset: u32, len: u32, memory: &[u8]) {
        self.global_memory.push(MemoryState {
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
                self.extern_names.insert(entity_index, name.to_string());
            }
        }
    }

    pub fn pre_opcode_state(
        &mut self,
        program_counter: u32,
        opcode: Instruction,
        stack: Vec<UntypedValue>,
        meta: &InstrMeta,
    ) {
        let memory_changes = take(&mut self.memory_changes);
        let stack = stack.iter().map(|v| v.to_bits()).collect();
        let opcode_state = OpCodeState {
            program_counter,
            opcode,
            memory_changes,
            stack,
            source_pc: meta.pos() as u32,
            code: meta.code(),
        };
        self.logs.push(opcode_state.clone());
    }

    pub fn function_call(
        &mut self,
        fn_index: u32,
        max_stack_height: usize,
        num_locals: usize,
        fn_name: String,
    ) {
        let resolved_name = self.extern_names.get(&fn_index).unwrap_or(&fn_name);
        self.fns_meta.push(FunctionMeta {
            fn_index,
            max_stack_height: max_stack_height as u32,
            num_locals: num_locals as u32,
            fn_name: resolved_name.clone(),
        })
    }

    pub fn global_variable(&mut self, value: UntypedValue, index: u32) {
        self.global_variables.push(GlobalVariable {
            value: value.to_bits(),
            index,
        })
    }

    pub fn memory_change(&mut self, offset: u32, len: u32, memory: &[u8]) {
        self.memory_changes.push(MemoryState {
            offset,
            len,
            data: Vec::from(memory),
        });
    }
}
