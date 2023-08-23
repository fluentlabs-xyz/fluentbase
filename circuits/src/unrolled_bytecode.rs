use crate::util::Field;
use fluentbase_rwasm::rwasm::{InstructionSet, ReducedModuleReader, ReducedModuleTrace};
use itertools::Itertools;
use std::marker::PhantomData;

#[derive(Clone, Default, Debug)]
pub struct UnrolledBytecode<F: Field> {
    read_traces: Vec<ReducedModuleTrace>,
    instruction_set: InstructionSet,
    _pd: PhantomData<F>,
}

impl<F: Field> UnrolledBytecode<F> {
    pub fn new(bytecode: &[u8]) -> Self {
        let mut module_reader = ReducedModuleReader::new(bytecode);
        let mut traces: Vec<ReducedModuleTrace> = Vec::new();
        loop {
            let trace = match module_reader.trace_opcode() {
                Some(trace) => trace,
                None => break,
            };
            traces.push(trace);
        }
        Self {
            read_traces: traces,
            instruction_set: module_reader.instruction_set,
            _pd: Default::default(),
        }
    }

    pub fn read_traces(&self) -> &Vec<ReducedModuleTrace> {
        &self.read_traces
    }

    pub fn hash_traces(&self) -> Vec<[F; 2]> {
        let mut res = Vec::new();
        for instr in self.instruction_set.instr() {
            let (code, aux) = (instr.code_value(), instr.aux_value().unwrap_or_default());
            res.push([F::from(code as u64), F::from(aux.to_bits())]);
        }
        res
    }

    pub fn code_hash(&self) -> F {
        let items = self.hash_traces().iter().flatten().copied().collect_vec();
        F::hash_msg(items.as_slice(), None)
    }
}
