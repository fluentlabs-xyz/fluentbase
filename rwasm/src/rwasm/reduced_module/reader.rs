use crate::{
    common::UntypedValue,
    engine::bytecode::{BranchOffset, InstrMeta, Instruction},
    rwasm::{BinaryFormat, BinaryFormatError, BinaryFormatReader, InstructionSet},
};
use alloc::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ReducedModuleTrace {
    pub offset: usize,
    pub code: u8,
    pub raw_bytes: Vec<u8>,
    pub aux_size: usize,
    pub aux: UntypedValue,
    pub instr: Result<Instruction, BinaryFormatError>,
}

pub struct ReducedModuleReader<'a> {
    pub binary_format_reader: BinaryFormatReader<'a>,
    pub instruction_set: InstructionSet,
    pub relative_position: BTreeMap<u32, u32>,
}

impl<'a> ReducedModuleReader<'a> {
    pub fn new(sink: &'a [u8]) -> Self {
        Self {
            binary_format_reader: BinaryFormatReader::new(sink),
            instruction_set: InstructionSet::new(),
            relative_position: BTreeMap::new(),
        }
    }

    pub fn read_all(sink: &[u8]) -> Result<InstructionSet, BinaryFormatError> {
        let mut reader = ReducedModuleReader::new(sink);
        reader.read_till_error()?;
        Ok(reader.instruction_set)
    }

    pub fn read_till_error(&mut self) -> Result<(), BinaryFormatError> {
        let mut last_trace: Option<ReducedModuleTrace> = None;
        loop {
            let trace = self.trace_opcode();
            if trace.is_none() {
                break;
            }
            last_trace = trace;
        }
        if let Some(last_trace) = last_trace {
            last_trace.instr?;
        }
        self.rewrite_offsets()?;
        Ok(())
    }

    pub fn trace_opcode(&mut self) -> Option<ReducedModuleTrace> {
        if self.binary_format_reader.is_empty() {
            // if reader is empty then we've reached end of the stream
            return None;
        } else if self.instruction_set.len() as usize != self.relative_position.len() {
            // if we have such mismatch then last record is error
            return None;
        }

        let pos_before = self.binary_format_reader.pos();

        let instr = Instruction::read_binary(&mut self.binary_format_reader);
        let aux = instr
            .map(|instr| instr.aux_value().unwrap_or_default())
            .unwrap_or_default();
        let pos_after = self.binary_format_reader.pos();

        let trace = ReducedModuleTrace {
            offset: pos_before,
            code: self.binary_format_reader.sink[pos_before],
            raw_bytes: self.binary_format_reader.sink[pos_before..pos_after].to_vec(),
            aux_size: pos_after - pos_before - 1,
            aux,
            instr,
        };

        self.relative_position
            .insert(trace.offset as u32, self.instruction_set.len());
        if let Ok(instr) = instr {
            self.instruction_set
                .push_with_meta(instr, InstrMeta::new(trace.offset, trace.code as u16));
        }

        Some(trace)
    }

    pub fn rewrite_offsets(&mut self) -> Result<(), BinaryFormatError> {
        for (index, opcode) in self.instruction_set.instr.iter_mut().enumerate() {
            if let Some(jump_offset) = opcode.get_jump_offset() {
                let relative_offset = self
                    .relative_position
                    .get(&(jump_offset.to_i32() as u32))
                    .ok_or(BinaryFormatError::ReachedUnreachable)?;
                opcode.update_branch_offset(BranchOffset::from(
                    *relative_offset as i32 - index as i32,
                ));
            }
        }
        Ok(())
    }
}
