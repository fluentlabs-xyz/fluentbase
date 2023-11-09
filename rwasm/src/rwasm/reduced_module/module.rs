use crate::{
    engine::bytecode::Instruction,
    module::{FuncIdx, FuncTypeIdx, MemoryIdx, ModuleBuilder, ModuleError, ModuleResources},
    rwasm::{
        instruction_set::InstructionSet,
        platform::ImportLinker,
        reduced_module::{
            reader::ReducedModuleReader,
            types::{ReducedModuleError, N_MAX_MEMORY_PAGES},
        },
    },
    Engine,
    FuncType,
    Module,
};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
};

pub struct ReducedModule {
    pub(crate) instruction_set: InstructionSet,
    relative_position: BTreeMap<u32, u32>,
}

impl ReducedModule {
    pub fn new(sink: &[u8]) -> Result<ReducedModule, ReducedModuleError> {
        let mut reader = ReducedModuleReader::new(sink);
        reader
            .read_till_error()
            .map_err(|e| ReducedModuleError::BinaryFormat(e))?;

        Ok(ReducedModule {
            instruction_set: reader.instruction_set,
            relative_position: reader.relative_position,
        })
    }

    pub fn bytecode(&self) -> &InstructionSet {
        &self.instruction_set
    }

    pub fn to_module(&self, engine: &Engine, import_linker: &ImportLinker) -> Module {
        let builder = self.to_module_builder(engine, import_linker, FuncType::new([], []));
        builder.finish()
    }

    pub fn to_module_builder<'a>(
        &'a self,
        engine: &'a Engine,
        import_linker: &ImportLinker,
        func_type: FuncType,
    ) -> ModuleBuilder {
        let mut builder = ModuleBuilder::new(engine);

        // main function has empty inputs and outputs
        let mut default_func_types = BTreeMap::new();
        let mut get_func_type_or_create =
            |func_type: FuncType, builder: &mut ModuleBuilder| -> FuncTypeIdx {
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
        get_func_type_or_create(func_type, &mut builder);

        let mut code_section = self.bytecode().clone();

        // find all used imports and map them
        let mut import_mapping = BTreeMap::new();
        for instr in code_section.instr.iter_mut() {
            let host_index = match instr {
                Instruction::Call(func) => func.to_u32(),
                _ => continue,
            };
            let func_index = import_mapping
                .get(&host_index)
                .copied()
                .unwrap_or(import_mapping.len() as u32);
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
        for instr in code_section.instr.iter_mut() {
            let func_offset = match instr {
                Instruction::RefFunc(func) => func.to_u32(),
                _ => continue,
            };
            if let Some(relative_pos) = self.relative_position.get(&func_offset) {
                instr.update_call_index(*relative_pos);
            }
        }

        // push main functions (we collapse all functions into one)
        builder
            .push_funcs(vec![Result::<FuncTypeIdx, ModuleError>::Ok(
                FuncTypeIdx::from(0),
            )])
            .unwrap();

        // mark headers for missing functions inside binary
        let resources = ModuleResources::new(&builder);
        let compiled_func = resources
            .get_compiled_func(FuncIdx::from(import_len))
            .unwrap();
        engine.init_func(
            compiled_func,
            0,
            0,
            code_section.instr.clone(),
            code_section.metas.clone().unwrap(),
        );
        // push segments
        let mut data_segments = BTreeSet::new();
        let mut elem_segments = BTreeSet::new();

        for instr in code_section.instr.iter() {
            match instr {
                Instruction::DataStore8(seg)
                | Instruction::DataStore16(seg)
                | Instruction::DataStore32(seg)
                | Instruction::DataStore64(seg) => {
                    data_segments.insert(seg.to_u32());
                }
                Instruction::ElemStore(seg) => {
                    elem_segments.insert(seg.to_u32());
                }
                _ => continue,
            }
        }
        if !data_segments.is_empty() {
            let max_data_segment = data_segments.iter().max().copied().unwrap_or_default() as usize;
            (0..=max_data_segment).for_each(|_| {
                builder.push_passive_data_segment();
            });
        }

        if !elem_segments.is_empty() {
            (0..=elem_segments.len()).for_each(|_| {
                builder.push_passive_elem_segment();
            })
        }
        // allocate default memory
        builder
            .push_default_memory(0, Some(N_MAX_MEMORY_PAGES))
            .unwrap();
        builder
            .push_export("memory".to_string().into_boxed_str(), MemoryIdx::from(0))
            .unwrap();
        // set 0 function as an entrypoint (it goes right after import section)
        let main_index = import_mapping.len() as u32;
        //builder.set_start(FuncIdx::from(main_index));
        builder
            .push_export(
                "main".to_string().into_boxed_str(),
                FuncIdx::from(main_index),
            )
            .unwrap();
        // push required amount of globals and tables
        let num_globals = self.bytecode().count_globals();
        builder.push_empty_globals(num_globals as usize).unwrap();
        let num_tables = self.bytecode().count_tables();
        builder.push_empty_tables(num_tables as usize).unwrap();
        // finalize module
        builder
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
