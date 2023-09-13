use crate::{
    arena::ArenaIndex,
    common::{UntypedValue, ValueType},
    engine::{
        bytecode::{BranchOffset, Instruction, TableIdx},
        code_map::InstructionPtr,
        DropKeep,
    },
    module::{ConstExpr, DataSegment, DataSegmentKind, ElementSegmentKind, ImportName, Imported},
    rwasm::{
        binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatWriter},
        instruction_set::InstructionSet,
        ImportLinker,
        MAX_MEMORY_PAGES,
    },
    Config,
    Engine,
    Module,
};
use alloc::{collections::BTreeMap, vec::Vec};
use core::ops::Deref;
use std::collections::HashSet;

mod drop_keep;

#[derive(Debug)]
pub enum CompilerError {
    ModuleError(crate::Error),
    MissingEntrypoint,
    MissingFunction,
    NotSupported(&'static str),
    OutOfBuffer,
    BinaryFormat(BinaryFormatError),
    NotSupportedImport,
    UnknownImport(ImportName),
    MemoryUsageTooBig,
}

pub trait Translator {
    fn translate(&self, result: &mut InstructionSet) -> Result<(), CompilerError>;
}

pub struct Compiler<'linker> {
    engine: Engine,
    module: Module,
    // translation state
    pub(crate) code_section: InstructionSet,
    function_mapping: BTreeMap<u32, u32>,
    import_linker: Option<&'linker ImportLinker>,
    function_dependencies: HashSet<u32>,
    is_translated: bool,
}

impl<'linker> Compiler<'linker> {
    pub fn new(wasm_binary: &[u8]) -> Result<Self, CompilerError> {
        Self::new_with_linker(wasm_binary, None)
    }

    pub fn new_with_linker(
        wasm_binary: &[u8],
        import_linker: Option<&'linker ImportLinker>,
    ) -> Result<Self, CompilerError> {
        let mut config = Config::default();
        config.consume_fuel(false);
        let engine = Engine::new(&config);
        let module =
            Module::new(&engine, wasm_binary).map_err(|e| CompilerError::ModuleError(e))?;
        Ok(Compiler {
            engine,
            module,
            code_section: InstructionSet::new(),
            function_mapping: BTreeMap::new(),
            import_linker,
            function_dependencies: HashSet::new(),
            is_translated: false,
        })
    }

    pub fn translate(&mut self, main_index: Option<u32>) -> Result<(), CompilerError> {
        if self.is_translated {
            unreachable!("already translated");
        }
        // translate globals, tables and memory
        let total_globals = self.module.globals.len();
        for i in 0..total_globals {
            self.translate_global(i as u32)?;
        }
        let total_tables = self.module.tables.len();
        for i in 0..total_tables {
            self.translate_table(i as u32)?;
        }
        self.translate_memory()?;
        // find main entrypoint (it must starts with `main` keyword)
        let main_index = if main_index.is_none() {
            self.module
                .exports
                .get("main")
                .ok_or(CompilerError::MissingEntrypoint)?
                .into_func_idx()
                .ok_or(CompilerError::MissingEntrypoint)?
        } else {
            main_index.unwrap()
        };
        // translate main entrypoint
        self.translate_function(main_index)?;
        // translate rest functions
        let total_fns = self.module.funcs.len();
        for i in 0..total_fns {
            if i != main_index as usize {
                self.translate_function(i as u32)?;
            }
        }
        // there is no need to inject because code is already validated
        self.code_section.finalize(false);
        self.is_translated = true;
        Ok(())
    }

    pub fn translate_wo_entrypoint(&mut self) -> Result<(), CompilerError> {
        if self.is_translated {
            unreachable!("already translated");
        }
        // translate memory and global first
        let total_globals = self.module.globals.len();
        for i in 0..total_globals {
            self.translate_global(i as u32)?;
        }
        self.translate_memory()?;
        // translate rest functions
        let total_fns = self.module.funcs.len();
        for i in 0..total_fns {
            self.translate_function(i as u32)?;
        }
        self.is_translated = true;
        Ok(())
    }

    fn read_memory_segment<'a>(
        memory: &DataSegment,
    ) -> Result<(UntypedValue, &[u8]), CompilerError> {
        match memory.kind() {
            DataSegmentKind::Active(seg) => {
                let data_offset = seg
                    .offset()
                    .eval_const()
                    .ok_or(CompilerError::NotSupported("can't eval offset"))?;
                if seg.memory_index().into_u32() != 0 {
                    return Err(CompilerError::NotSupported("not zero index"));
                }
                Ok((data_offset, memory.bytes()))
            }
            DataSegmentKind::Passive => {
                Err(CompilerError::NotSupported("passive mode is not supported"))
            }
        }
    }

    fn translate_memory(&mut self) -> Result<(), CompilerError> {
        let mut init_memory = 0;
        for memory in self.module.data_segments.iter() {
            let (offset, bytes) = Self::read_memory_segment(memory)?;
            init_memory += offset.to_bits() as u32 + bytes.len() as u32;
        }
        const PAGE_SIZE: u32 = 65536;
        let default_pages = (init_memory + PAGE_SIZE - 1) / PAGE_SIZE;
        if default_pages > MAX_MEMORY_PAGES {
            return Err(CompilerError::MemoryUsageTooBig);
        }
        for memory in self.module.data_segments.iter() {
            let (offset, bytes) = Self::read_memory_segment(memory)?;
            self.code_section.add_memory(offset.to_bits() as u32, bytes);
        }
        Ok(())
    }

    fn translate_global(&mut self, global_index: u32) -> Result<(), CompilerError> {
        let len_imported = self.module.imports.len_globals;
        let globals = &self.module.globals[len_imported..];
        assert!(global_index < globals.len() as u32);
        let global_inits = &self.module.globals_init;
        assert!(global_index < global_inits.len() as u32);
        let global_expr = &global_inits[global_index as usize];
        if let Some(value) = global_expr.eval_const() {
            self.code_section.op_i64_const(value);
        } else if let Some(value) = global_expr.funcref() {
            self.code_section.op_ref_func(value.into_u32());
        }
        self.code_section.op_global_set(global_index);
        Ok(())
    }

    fn translate_const_expr(&self, const_expr: &ConstExpr) -> Result<UntypedValue, CompilerError> {
        let init_value = const_expr.eval_const().ok_or(CompilerError::NotSupported(
            "only static global variables supported",
        ))?;
        Ok(init_value)
    }

    fn translate_table(&mut self, table_index: u32) -> Result<(), CompilerError> {
        assert!(table_index < self.module.tables.len() as u32);
        let table = &self.module.tables[table_index as usize];
        if table.element() != ValueType::FuncRef {
            return Err(CompilerError::NotSupported(
                "only funcref type is supported for tables",
            ));
        }
        let mut table_init_size = 0;
        for e in self.module.element_segments.iter() {
            let aes = match &e.kind {
                ElementSegmentKind::Passive | ElementSegmentKind::Declared => {
                    return Err(CompilerError::NotSupported(
                        "passive or declared mode for element segments is not supported",
                    ))
                }
                ElementSegmentKind::Active(aes) => aes,
            };
            if aes.table_index().into_u32() != table_index {
                continue;
            }
            if e.ty != ValueType::FuncRef {
                return Err(CompilerError::NotSupported(
                    "only funcref type is supported for element segments",
                ));
            }
            table_init_size += e.items.items().len();
        }
        self.code_section.op_ref_func(0);
        self.code_section.op_i64_const(table_init_size);
        self.code_section.op_table_grow(table_index);
        self.code_section.op_drop();
        for e in self.module.element_segments.iter() {
            let aes = match &e.kind {
                ElementSegmentKind::Passive | ElementSegmentKind::Declared => {
                    return Err(CompilerError::NotSupported(
                        "passive or declared mode for element segments is not supported",
                    ))
                }
                ElementSegmentKind::Active(aes) => aes,
            };
            if aes.table_index().into_u32() != table_index {
                continue;
            }
            if e.ty != ValueType::FuncRef {
                return Err(CompilerError::NotSupported(
                    "only funcref type is supported for element segments",
                ));
            }
            let table_idx = self.translate_const_expr(aes.offset())?;
            for (index, item) in e.items.items().iter().enumerate() {
                self.code_section.op_i32_const(index as u32);
                if let Some(value) = item.eval_const() {
                    self.code_section.op_i64_const(value);
                } else if let Some(value) = item.funcref() {
                    self.code_section.op_ref_func(value.into_u32());
                }
                self.code_section.op_table_set(table_idx.to_bits() as u32);
            }
        }
        Ok(())
    }

    fn translate_function(&mut self, fn_index: u32) -> Result<(), CompilerError> {
        let import_len = self.module.imports.len_funcs;
        // don't translate import functions because we can't translate them
        if fn_index < import_len as u32 {
            return Ok(());
        }
        let fn_index = fn_index - import_len as u32;
        let func_body = self
            .module
            .compiled_funcs
            .get(fn_index as usize)
            .ok_or(CompilerError::MissingFunction)?;
        let beginning_offset = self.code_section.len();
        // ....
        // let func_type = self.module.funcs[*fn_index as usize + import_len];
        // let func_type = self.engine.resolve_func_type(&func_type, Clone::clone);
        // let num_inputs = func_type.params();
        // let num_outputs = func_type.results();

        // reserve stack for locals
        let len_locals = self.engine.num_locals(*func_body);
        (0..len_locals).for_each(|_| {
            self.code_section.op_i32_const(0);
        });
        // translate instructions
        let (mut instr_ptr, instr_end) = self.engine.instr_ptr(*func_body);
        while instr_ptr != instr_end {
            self.translate_opcode(&mut instr_ptr)?;
        }
        // remember function offset in the mapping
        self.function_mapping.insert(fn_index, beginning_offset);
        Ok(())
    }

    fn extract_drop_keep(instr_ptr: &mut InstructionPtr) -> DropKeep {
        instr_ptr.add(1);
        let next_instr = instr_ptr.get();
        match next_instr {
            Instruction::Return(drop_keep) => *drop_keep,
            _ => unreachable!("incorrect instr after break adjust ({:?})", *next_instr),
        }
    }

    fn extract_table(instr_ptr: &mut InstructionPtr) -> TableIdx {
        instr_ptr.add(1);
        let next_instr = instr_ptr.get();
        match next_instr {
            Instruction::TableGet(table_idx) => *table_idx,
            _ => unreachable!("incorrect instr after break adjust ({:?})", *next_instr),
        }
    }

    fn translate_opcode(&mut self, instr_ptr: &mut InstructionPtr) -> Result<(), CompilerError> {
        use Instruction as WI;
        match *instr_ptr.get() {
            WI::BrAdjust(branch_offset) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                self.code_section.op_br(branch_offset);
                self.code_section.op_return();
            }
            WI::BrAdjustIfNez(branch_offset) => {
                let br_if_offset = self.code_section.len();
                self.code_section.op_br_if_eqz(0);
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                let drop_keep_len = self.code_section.len() - br_if_offset - 1;
                self.code_section
                    .get_mut(br_if_offset as usize)
                    .unwrap()
                    .update_branch_offset(BranchOffset::from(1 + drop_keep_len as i32));
                self.code_section.op_br(branch_offset);
                self.code_section.op_return();
            }
            WI::ReturnCallInternal(func_idx) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                let fn_index = func_idx.into_usize() as u32;
                self.code_section.op_return_call_internal(fn_index);
                self.code_section.op_return();
                self.function_dependencies.insert(func_idx.to_u32());
            }
            WI::ReturnCall(_func) => {
                unreachable!("wait, should it call translate host call?");
                // Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                // self.code_section.op_return_call(func);
                // self.code_section.op_return();
            }
            WI::ReturnCallIndirect(_) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                let table_idx = Self::extract_table(instr_ptr);
                self.code_section.op_return_call_indirect(table_idx);
                self.code_section.op_return();
            }
            WI::Return(drop_keep) => {
                drop_keep.translate(&mut self.code_section)?;
                self.code_section.op_return();
            }
            WI::ReturnIfNez(drop_keep) => {
                let br_if_offset = self.code_section.len();
                self.code_section.op_br_if_eqz(0);
                drop_keep.translate(&mut self.code_section)?;
                let drop_keep_len = self.code_section.len() - br_if_offset - 1;
                self.code_section
                    .get_mut(br_if_offset as usize)
                    .unwrap()
                    .update_branch_offset(BranchOffset::from(1 + drop_keep_len as i32));
                self.code_section.op_return_if_nez();
            }
            WI::CallInternal(func_idx) => {
                let fn_index = func_idx.into_usize() as u32;
                self.code_section.op_call_internal(fn_index);
                self.function_dependencies.insert(func_idx.to_u32());
            }
            WI::CallIndirect(_) => {
                let table_idx = Self::extract_table(instr_ptr);
                self.code_section.op_call_indirect(table_idx);
            }
            WI::Call(func_idx) => {
                self.translate_host_call(func_idx.to_u32())?;
            }
            WI::ConstRef(const_ref) => {
                let resolved_const = self.engine.resolve_const(const_ref).unwrap();
                self.code_section.op_i64_const(resolved_const);
            }
            _ => {
                self.code_section.push(*instr_ptr.get());
            }
        };
        instr_ptr.add(1);
        Ok(())
    }

    fn translate_host_call(&mut self, fn_index: u32) -> Result<(), CompilerError> {
        let imports = self.module.imports.items.deref();
        if fn_index >= imports.len() as u32 {
            return Err(CompilerError::NotSupportedImport);
        }
        let imported = &imports[fn_index as usize];
        let import_name = match imported {
            Imported::Func(import_name) => import_name,
            _ => return Err(CompilerError::NotSupportedImport),
        };
        let import_index = self
            .import_linker
            .ok_or(CompilerError::UnknownImport(import_name.clone()))?
            .index_mapping()
            .get(import_name)
            .ok_or(CompilerError::UnknownImport(import_name.clone()))?;
        self.code_section.op_call(*import_index);
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>, CompilerError> {
        if !self.is_translated {
            self.translate(None)?;
        }
        let bytecode = &mut self.code_section;

        let mut states: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        let mut buffer_offset = 0u32;
        for code in bytecode.instr.iter() {
            let mut buffer: [u8; 100] = [0; 100];
            let mut binary_writer = BinaryFormatWriter::new(&mut buffer[..]);
            code.write_binary(&mut binary_writer)
                .map_err(|e| CompilerError::BinaryFormat(e))?;
            let buffer = binary_writer.to_vec();
            let buffer_size = buffer.len() as u32;
            states.push((buffer_offset, buffer_size, buffer));
            buffer_offset += buffer_size;
        }

        for (i, code) in bytecode.instr.iter().enumerate() {
            let mut code = code.clone();
            let mut affected = false;
            match code {
                Instruction::CallInternal(func) | Instruction::ReturnCallInternal(func) => {
                    let func_offset = self
                        .function_mapping
                        .get(&func.to_u32())
                        .ok_or(CompilerError::MissingFunction)?;
                    let state = &states[*func_offset as usize];
                    code.update_call_index(state.0);
                    affected = true;
                }
                Instruction::RefFunc(func_idx) => {
                    let func_offset = self
                        .function_mapping
                        .get(&func_idx.to_u32())
                        .ok_or(CompilerError::MissingFunction)?;
                    let state = &states[*func_offset as usize];
                    code.update_call_index(state.0);
                    affected = true;
                }
                _ => {}
            };
            if let Some(jump_offset) = code.get_jump_offset() {
                let jump_label = (jump_offset.to_i32() + i as i32) as usize;
                let target_state = states.get(jump_label).ok_or(CompilerError::OutOfBuffer)?;
                code.update_branch_offset(BranchOffset::from(target_state.0 as i32));
                affected = true;
            }
            if affected {
                let current_state = states.get_mut(i).ok_or(CompilerError::OutOfBuffer)?;
                current_state.2.clear();
                code.write_binary_to_vec(&mut current_state.2)
                    .map_err(|e| CompilerError::BinaryFormat(e))?;
            }
        }

        let res = states.iter().fold(Vec::new(), |mut res, state| {
            res.extend(&state.2);
            res
        });
        Ok(res)
    }
}
