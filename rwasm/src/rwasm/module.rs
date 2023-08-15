use crate::{
    engine::bytecode::{BranchOffset, InstrMeta, Instruction},
    module::{FuncIdx, FuncTypeIdx, MemoryIdx, ModuleBuilder, ModuleError, ModuleResources},
    rwasm::{
        binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatReader},
        instruction_set::InstructionSet,
        platform::ImportLinker,
    },
    Engine,
    FuncType,
    Module,
};
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

#[derive(Debug)]
pub enum ReducedModuleError {
    MissingEntrypoint,
    NotSupportedOpcode,
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
    relative_position: BTreeMap<u32, u32>,
}

impl ReducedModule {
    pub fn new(sink: &[u8]) -> Result<ReducedModule, ReducedModuleError> {
        let mut reader = BinaryFormatReader::new(sink);

        let mut instruction_set = InstructionSet::new();

        // here we store mapping from jump destination to the opcode offset
        let mut relative_position: BTreeMap<u32, u32> = BTreeMap::new();

        // read all opcodes from binary
        while !reader.is_empty() {
            let offset = reader.pos();
            let code = reader.sink[0] as u16;

            let instr = Instruction::read_binary(&mut reader).map_err(|e| ReducedModuleError::BinaryFormat(e))?;

            relative_position.insert(offset as u32, instruction_set.len());
            instruction_set.push_with_meta(instr, InstrMeta::new(offset, code));
        }

        // if instruction has jump offset then its br-like and we should re-write jump offset
        for (index, opcode) in instruction_set.instr.iter_mut().enumerate() {
            if let Some(jump_offset) = opcode.get_jump_offset() {
                let relative_offset = relative_position
                    .get(&(jump_offset.to_i32() as u32))
                    .ok_or(ReducedModuleError::ReachedUnreachable)?;
                opcode.update_branch_offset(BranchOffset::from(*relative_offset as i32 - index as i32));
            }
        }

        Ok(ReducedModule {
            instruction_set,
            relative_position,
        })
    }

    pub fn bytecode(&self) -> &InstructionSet {
        &self.instruction_set
    }

    pub fn to_module(&self, engine: &Engine, import_linker: &ImportLinker) -> Result<Module, ModuleError> {
        let builder = self.to_module_builder(engine, import_linker)?;
        Ok(builder.finish())
    }

    pub fn to_module_builder<'a>(
        &'a self,
        engine: &'a Engine,
        import_linker: &ImportLinker,
    ) -> Result<ModuleBuilder, ModuleError> {
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
        for instr in code_section.instr.iter_mut() {
            let host_index = match instr {
                Instruction::Call(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = import_mapping.len() as u32;
            instr.update_call_index(func_index);
            let import_func = import_linker
                .resolve_by_index(host_index)
                .ok_or_else(|| unreachable!("unknown host index: ({:?})", host_index))
                .unwrap();
            let func_type = import_func.func_type().clone();
            let func_type_idx = get_func_type_or_create(func_type, &mut builder);
            if !import_mapping.contains_key(&host_index) {
                import_mapping.insert(host_index, func_index);
                builder
                    .push_function_import(import_func.import_name().clone(), func_type_idx)
                    .unwrap();
            }
        }
        let import_len = import_mapping.len() as u32;

        // reconstruct all functions and fix bytecode calls
        let mut func_index_offset = BTreeMap::new();
        func_index_offset.insert(import_len, 0);
        for instr in code_section.instr.iter_mut() {
            let func_offset = match instr {
                Instruction::CallInternal(func) => func.to_u32(),
                Instruction::ReturnCallInternal(func) => func.to_u32(),
                Instruction::RefFunc(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = func_index_offset.len() as u32;
            instr.update_call_index(func_index);
            let relative_pos = self.relative_position.get(&func_offset).unwrap();
            func_index_offset.insert(func_index + import_len, *relative_pos);
        }

        // push main functions (we collapse all functions into one)
        let funcs: Vec<Result<FuncTypeIdx, ModuleError>> = func_index_offset
            .iter()
            .map(|_| Result::<FuncTypeIdx, ModuleError>::Ok(FuncTypeIdx::from(0)))
            .collect();
        builder.push_funcs(funcs)?;

        // mark headers for missing functions inside binary
        let resources = ModuleResources::new(&builder);
        for (func_index, func_offset) in func_index_offset.iter() {
            let compiled_func = resources.get_compiled_func(FuncIdx::from(*func_index)).unwrap();
            if *func_offset == 0 || compiled_func.to_u32() == 0 {
                // TODO: "what if we don't have main function? it might happen in e2e tests"
                //assert_eq!(compiled_func.to_u32(), 0, "main function doesn't have zero offset");
            }
            // function main has 0 offset, init bytecode for entire section
            if *func_offset == 0 {
                engine.init_func(compiled_func, 0, 0, code_section.instr.clone());
            } else {
                engine.mark_func(compiled_func, 0, 0, *func_offset as usize);
            }
        }
        // allocate default memory
        builder.push_default_memory(MAX_MEMORY_PAGES, Some(MAX_MEMORY_PAGES))?;
        builder.push_export("memory".to_string().into_boxed_str(), MemoryIdx::from(0))?;
        // set 0 function as an entrypoint (it goes right after import section)
        let main_index = import_mapping.len() as u32;
        builder.set_start(FuncIdx::from(main_index));
        builder.push_export("main".to_string().into_boxed_str(), FuncIdx::from(main_index))?;
        // push required amount of globals
        let num_globals = self.bytecode().count_globals();
        builder.push_empty_i64_globals(num_globals as usize)?;
        // finalize module
        Ok(builder)
    }

    pub fn trace_binary(&self) -> String {
        let mut result = String::new();
        for opcode in self.bytecode().instr.iter() {
            let str = format!("{:?}\n", opcode);
            result += str.as_str();
        }
        result
    }
}
