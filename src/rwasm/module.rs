use crate::common::ValueType;
use crate::module::{translate, FuncIdx, FuncTypeIdx, ModuleBuilder, ModuleError};
use crate::rwasm::instruction_set::InstructionSet;
use crate::{
    engine::bytecode::{BranchOffset, InstrMeta, Instruction},
    rwasm::binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatReader},
    Engine, FuncType, Module,
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use wasmparser::FunctionBody;

#[derive(Debug)]
pub enum ReducedModuleError {
    MissingEntrypoint,
    NotSupportedOpcode,
    MissingFunction,
    NotSupportedImport,
    NotSupportedMemory(&'static str),
    ParseError(&'static str),
    OutOfBuffer,
    ReachedUnreachable,
    IllegalOpcode(u8),
    ImpossibleJump,
    InternalError(&'static str),
    MemoryOverflow,
    EmptyBytecode,
    BinaryFormat(BinaryFormatError),
}

pub struct ReducedModule {
    pub(crate) instruction_set: InstructionSet,
    metas: Vec<InstrMeta>,
    relative_position: BTreeMap<u32, u32>,
    num_globals: u32,
}

impl ReducedModule {
    pub fn new(sink: &[u8]) -> Result<ReducedModule, ReducedModuleError> {
        let mut reader = BinaryFormatReader::new(sink);

        let mut instruction_set = InstructionSet::new();
        let mut metas = Vec::new();

        // here we store mapping from jump destination to the opcode offset
        let mut relative_position: BTreeMap<u32, u32> = BTreeMap::new();

        // read all opcodes from binary
        while !reader.is_empty() {
            let offset = reader.pos();
            let code = reader.sink[0];

            let instr = Instruction::read_binary(&mut reader).map_err(|e| ReducedModuleError::BinaryFormat(e))?;
            // println!("{:#04x}: {:?}", offset, instr);

            relative_position.insert(offset as u32, instruction_set.len());
            instruction_set.push(instr);
            metas.push(InstrMeta::new(offset, code));
        }
        // println!();

        // if instruction has jump offset then its br-like and we should re-write jump offset
        for (index, opcode) in instruction_set.0.iter_mut().enumerate() {
            if let Some(jump_offset) = opcode.get_jump_offset() {
                let relative_offset = relative_position
                    .get(&(jump_offset.to_i32() as u32))
                    .ok_or(ReducedModuleError::ReachedUnreachable)?;
                opcode.update_branch_offset(BranchOffset::from(*relative_offset as i32 - index as i32));
            }
        }

        let num_globals = instruction_set
            .0
            .iter()
            .filter_map(|opcode| match opcode {
                Instruction::GlobalGet(index) | Instruction::GlobalSet(index) => Some(index.to_u32()),
                _ => None,
            })
            .max()
            .map(|v| v + 1)
            .unwrap_or_default();

        Ok(ReducedModule {
            instruction_set,
            metas,
            relative_position,
            num_globals,
        })
    }

    pub fn bytecode(&self) -> &InstructionSet {
        &self.instruction_set
    }

    pub fn metas(&self) -> &Vec<InstrMeta> {
        &self.metas
    }

    pub fn to_module(&self, engine: &Engine) -> Result<Module, ModuleError> {
        let mut builder = ModuleBuilder::new(engine);
        // main function has empty inputs and outputs
        let main_func_type = Result::<FuncType, ModuleError>::Ok(FuncType::new([], []));
        builder.push_func_types(vec![main_func_type])?;
        // reconstruct all functions and fix bytecode calls
        let mut code_section = self.bytecode().clone();
        let mut func_index_offset = BTreeMap::new();
        func_index_offset.insert(0, 0);
        for instr in code_section.0.iter_mut() {
            let func_offset = match instr {
                Instruction::CallInternal(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = func_index_offset.len() as u32;
            instr.update_call_index(func_index);
            let relative_pos = self.relative_position.get(&func_offset).unwrap();
            func_index_offset.insert(func_index, *relative_pos);
        }
        // mark headers for missing functions inside binary
        let funcs: Vec<Result<FuncTypeIdx, ModuleError>> = func_index_offset
            .iter()
            .map(|_| Result::<FuncTypeIdx, ModuleError>::Ok(FuncTypeIdx::from(0)))
            .collect();
        builder.push_funcs(funcs)?;
        for (func_index, func_offset) in func_index_offset.iter() {
            let last_compiled_func = builder.compiled_funcs.get(*func_index as usize).unwrap();
            assert_eq!(last_compiled_func.to_u32(), *func_index);
            // for 0 function (main) init with entire bytecode section
            if *func_index == 0 {
                engine.init_func(*last_compiled_func, 0, 0, code_section.0.clone());
            } else {
                engine.mark_func(*last_compiled_func, 0, 0, *func_offset as usize);
            }
        }
        // set 0 function as an entrypoint
        builder.set_start(FuncIdx::from(0));
        // push required amount of globals
        builder.push_empty_i64_globals(self.num_globals as usize)?;
        // finalize module
        let module = builder.finish();
        Ok(module)
    }

    pub fn trace_binary(&self) -> String {
        let mut result = String::new();
        for opcode in self.bytecode().0.iter() {
            let str = format!("{:?}\n", opcode);
            result += str.as_str();
        }
        result
    }
}
