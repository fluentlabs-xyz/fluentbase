use crate::{
    common::UntypedValue,
    engine::bytecode::{InstrMeta, Instruction},
    rwasm::{BinaryFormat, BinaryFormatError, BinaryFormatReader, InstructionSet},
};
use alloc::{collections::BTreeMap, vec::Vec};

#[derive(Debug, Clone)]
pub struct ReducedModuleTrace {
    pub offset: usize,
    pub bytecode_length: usize,
    pub code: u8,
    pub raw_bytes: Vec<u8>,
    pub aux_size: usize,
    pub aux: UntypedValue,
    pub instr: Result<Instruction, BinaryFormatError>,
}

impl ReducedModuleTrace {
    pub fn raw_bytes_padded(&self, pad_length: usize) -> Vec<u8> {
        if self.raw_bytes.len() % pad_length == 0 {
            return self.raw_bytes.clone();
        }
        let add_bytes = pad_length - self.raw_bytes.len() % pad_length;
        let mut padded_bytes = self.raw_bytes.clone();
        padded_bytes.resize(padded_bytes.len() + add_bytes, 0);
        padded_bytes
    }
}

pub struct ReducedModuleReader<'a> {
    pub binary_format_reader: BinaryFormatReader<'a>,
    pub instruction_set: InstructionSet,
    pub relative_position: BTreeMap<u32, u32>,
    pub bytecode_length: usize,
}

impl<'a> ReducedModuleReader<'a> {
    pub fn new(sink: &'a [u8]) -> Self {
        Self {
            binary_format_reader: BinaryFormatReader::new(sink),
            instruction_set: InstructionSet::new(),
            relative_position: BTreeMap::new(),
            bytecode_length: sink.len(),
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
            bytecode_length: self.bytecode_length,
            code: self.binary_format_reader.sink[pos_before],
            raw_bytes: self.binary_format_reader.sink[pos_before..pos_after].to_vec(),
            aux_size: pos_after - pos_before - 1,
            aux,
            instr,
        };

        self.relative_position
            .insert(trace.offset as u32, self.instruction_set.len());
        if let Ok(instr) = instr {
            self.instruction_set.push_with_meta(
                instr,
                InstrMeta::new(
                    trace.offset,
                    trace.code as u16,
                    self.instruction_set.len() as usize,
                ),
            );
        }

        Some(trace)
    }
}
