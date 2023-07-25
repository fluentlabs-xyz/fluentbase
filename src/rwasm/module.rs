use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::module::ModuleResources;
use crate::rwasm::platform::ImportLinker;
use crate::{
    engine::bytecode::{BranchOffset, InstrMeta, Instruction},
    module::{FuncIdx, FuncTypeIdx, ModuleBuilder, ModuleError},
    rwasm::binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatReader},
    rwasm::instruction_set::InstructionSet,
    Engine, FuncType, Module,
};

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

const MAX_MEMORY_PAGES: u32 = 512;

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

    pub fn to_module(&self, engine: &Engine, import_linker: &mut ImportLinker) -> Result<Module, ModuleError> {
        let mut builder = ModuleBuilder::new(engine);

        // main function has empty inputs and outputs
        let mut default_func_types = BTreeMap::new();
        let mut get_func_type_or_create = |func_type: FuncType, builder: &mut ModuleBuilder| -> FuncTypeIdx {
            let func_type_idx = default_func_types.get(&func_type);
            let func_type_idx = if let Some(idx) = func_type_idx {
                *idx
            } else {
                let idx = default_func_types.len();
                default_func_types.insert(func_type.clone(), idx);
                builder.push_func_type(func_type).unwrap();
                idx
            };
            FuncTypeIdx::from(func_type_idx as u32)
        };
        get_func_type_or_create(FuncType::new([], []), &mut builder);

        let mut code_section = self.bytecode().clone();

        // find all used imports and map them
        let mut import_mapping = BTreeMap::new();
        for instr in code_section.0.iter_mut() {
            let host_index = match instr {
                Instruction::Call(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = import_mapping.len() as u32;
            import_mapping.insert(host_index, func_index);
            instr.update_call_index(func_index);
            let import_func = import_linker
                .resolve_by_index(host_index)
                .ok_or_else(|| unreachable!("unknown host index: ({:?})", host_index))
                .unwrap();
            let func_type = import_func.func_type().clone();
            let func_type_idx = get_func_type_or_create(func_type, &mut builder);
            builder
                .push_function_import(import_func.import_name().clone(), func_type_idx)
                .unwrap();
        }

        // reconstruct all functions and fix bytecode calls
        let mut func_index_offset = BTreeMap::new();
        func_index_offset.insert(import_mapping.len() as u32, 0);
        for instr in code_section.0.iter_mut() {
            let func_offset = match instr {
                Instruction::CallInternal(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = import_mapping.len() as u32 + func_index_offset.len() as u32;
            instr.update_call_index(func_index);
            let relative_pos = self.relative_position.get(&func_offset).unwrap();
            func_index_offset.insert(func_index, *relative_pos);
        }

        // push main functions
        let funcs: Vec<Result<FuncTypeIdx, ModuleError>> = func_index_offset
            .iter()
            .map(|_| Result::<FuncTypeIdx, ModuleError>::Ok(FuncTypeIdx::from(0)))
            .collect();
        builder.push_funcs(funcs)?;

        // mark headers for missing functions inside binary
        let mut resources = ModuleResources::new(&builder);
        for (func_index, func_offset) in func_index_offset.iter() {
            let compiled_func = resources.get_compiled_func(FuncIdx::from(*func_index)).unwrap();
            // for 0 function (main) init with entire bytecode section
            if compiled_func.to_u32() == 0 {
                engine.init_func(compiled_func, 0, 0, code_section.0.clone());
            } else {
                engine.mark_func(compiled_func, 0, 0, *func_offset as usize);
            }
        }
        // allocate default memory
        builder.push_default_memory(MAX_MEMORY_PAGES, Some(MAX_MEMORY_PAGES))?;
        // set 0 function as an entrypoint (it goes right after import section)
        builder.set_start(FuncIdx::from(import_mapping.len() as u32));
        // push required amount of globals
        builder.push_empty_i64_globals(self.num_globals as usize)?;
        // finalize module
        let mut module = builder.finish();
        // module.set_rwasm();
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
