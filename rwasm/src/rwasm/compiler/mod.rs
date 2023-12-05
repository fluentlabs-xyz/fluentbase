use crate::{
    arena::ArenaIndex,
    engine::{
        bytecode::{BranchOffset, Instruction, LocalDepth, TableIdx},
        code_map::InstructionPtr,
        DropKeep,
    },
    module::{ConstExpr, DataSegment, DataSegmentKind, ElementSegmentKind, ImportName, Imported},
    rwasm::{
        binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatWriter},
        compiler::drop_keep::{translate_drop_keep, DropKeepWithReturnParam},
        instruction_set::InstructionSet,
        FuncOrExport::Global,
        ImportLinker,
    },
    Config,
    Engine,
    FuncRef,
    FuncType,
    Module,
    Value,
};
use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};
use core::ops::Deref;
use fluentbase_rwasm_core::common::{Pages, UntypedValue, ValueType, F32};
use std::cell::RefCell;

mod drop_keep;
use crate::value::WithType;

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
    DropKeepOutOfBounds,
}

impl Into<i32> for CompilerError {
    fn into(self) -> i32 {
        match self {
            CompilerError::ModuleError(_) => -1,
            CompilerError::MissingEntrypoint => -2,
            CompilerError::MissingFunction => -3,
            CompilerError::NotSupported(_) => -4,
            CompilerError::OutOfBuffer => -5,
            CompilerError::BinaryFormat(_) => -6,
            CompilerError::NotSupportedImport => -7,
            CompilerError::UnknownImport(_) => -8,
            CompilerError::MemoryUsageTooBig => -9,
            CompilerError::DropKeepOutOfBounds => -10,
        }
    }
}

pub trait Translator {
    fn translate(&self, result: &mut InstructionSet) -> Result<(), CompilerError>;
}

pub struct CompilerConfig {
    pub fuel_consume: bool,
    pub tail_call: bool,
    pub extended_const: bool,
    pub translate_sections: bool,
    pub with_state: bool,
    pub translate_func_as_inline: bool,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            fuel_consume: true,
            tail_call: true,
            extended_const: true,
            translate_sections: true,
            with_state: false,
            translate_func_as_inline: false,
        }
    }
}

impl CompilerConfig {
    pub fn fuel_consume(mut self, value: bool) -> Self {
        self.fuel_consume = value;

        self
    }

    pub fn tail_call(mut self, value: bool) -> Self {
        self.tail_call = value;

        self
    }

    pub fn extended_const(mut self, value: bool) -> Self {
        self.extended_const = value;

        self
    }

    pub fn translate_sections(mut self, value: bool) -> Self {
        self.translate_sections = value;

        self
    }

    pub fn with_state(mut self, value: bool) -> Self {
        self.with_state = value;

        self
    }

    pub fn translate_func_as_inline(mut self, value: bool) -> Self {
        self.translate_func_as_inline = value;

        self
    }
}

pub struct Compiler<'linker> {
    engine: Engine,
    module: Module,
    // translation state
    pub(crate) code_section: InstructionSet,
    // mapping from function index to its position inside code section
    function_beginning: BTreeMap<u32, u32>,
    import_linker: Option<&'linker ImportLinker>,
    is_translated: bool,
    injection_segments: Vec<Injection>,
    br_table_status: Option<BrTableStatus>,
    func_type_check_idx: Rc<RefCell<Vec<FuncType>>>,
    global_start_index: u32,
    pub config: CompilerConfig,
}

const REF_FUNC_FUNCTION_OFFSET: u32 = 0xff000000;

#[derive(Debug)]
pub struct Injection {
    pub begin: i32,
    pub end: i32,
    pub origin_len: i32,
}

#[derive(Debug)]
struct BrTableStatus {
    injection_instructions: Vec<Instruction>,
    instr_countdown: u32,
}

#[derive(Debug)]
pub enum FuncOrExport {
    Export(&'static str),
    Func(u32),
    Global(Instruction),
    StateRouter(Vec<FuncOrExport>, RouterInstructions),
}

#[derive(Debug)]
pub struct RouterInstructions {
    pub state_ix: Instruction,
    pub input_ix: Vec<Instruction>,
    pub output_ix: Vec<Instruction>,
}

impl Default for FuncOrExport {
    fn default() -> Self {
        Self::Export("main")
    }
}

impl<'linker> Compiler<'linker> {
    pub fn new(wasm_binary: &[u8], config: CompilerConfig) -> Result<Self, CompilerError> {
        Self::new_with_linker(wasm_binary, config, None)
    }

    pub fn new_with_linker(
        wasm_binary: &[u8],
        config: CompilerConfig,
        import_linker: Option<&'linker ImportLinker>,
    ) -> Result<Self, CompilerError> {
        let mut engine_config = Config::default();
        engine_config.consume_fuel(config.fuel_consume);
        engine_config.wasm_tail_call(config.tail_call);
        engine_config.wasm_extended_const(config.extended_const);

        let engine = Engine::new(&engine_config);
        let module =
            Module::new(&engine, wasm_binary).map_err(|e| CompilerError::ModuleError(e))?;
        Ok(Compiler {
            engine,
            module,
            code_section: InstructionSet::new(),
            function_beginning: BTreeMap::new(),
            import_linker,
            is_translated: false,
            injection_segments: vec![],
            br_table_status: None,
            func_type_check_idx: Default::default(),
            global_start_index: 0,
            config,
        })
    }

    pub fn translate_func_as_inline(&mut self, v: bool) {
        self.config.translate_func_as_inline = v;
    }

    pub fn set_global_start_index(&mut self, idx: u32) {
        self.global_start_index = idx;
    }

    pub fn set_func_type_check_idx(&mut self, func_type_check_idx: Rc<RefCell<Vec<FuncType>>>) {
        self.func_type_check_idx = func_type_check_idx;
    }

    pub fn set_state(&mut self, with_state: bool) {
        self.config.with_state = with_state;
    }

    pub fn translate(&mut self, main_index: Option<FuncOrExport>) -> Result<(), CompilerError> {
        if self.is_translated {
            unreachable!("already translated");
        }
        // first we must translate all sections, this is an entrypoint

        if self.config.with_state {
            self.translate_sections_with_state(main_index.unwrap_or_default())?;
        } else if self.config.translate_sections {
            self.translate_sections(main_index.unwrap_or_default())?;
        }

        // translate rest functions
        let total_fns = self.module.funcs.len();
        for i in 0..total_fns {
            self.translate_function(i as u32)?;
        }
        // there is no need to inject because code is already validated
        self.code_section.finalize(false);
        self.is_translated = true;
        Ok(())
    }

    fn resolve_export_index(&self, name: &str) -> Result<u32, CompilerError> {
        let main_index = self
            .module
            .exports
            .get(name)
            .ok_or(CompilerError::MissingEntrypoint)?
            .into_func_idx()
            .ok_or(CompilerError::MissingEntrypoint)?;
        Ok(main_index)
    }

    fn resolve_func_index(&self, export: &FuncOrExport) -> Result<Option<u32>, CompilerError> {
        match export {
            FuncOrExport::Export(name) => Some(self.resolve_export_index(name)).transpose(),
            FuncOrExport::Func(index) => Ok(Some(*index)),
            _ => Ok(None),
        }
    }

    fn resolve_global_ix(&self, export: &FuncOrExport) -> Option<Instruction> {
        match export {
            FuncOrExport::Global(ix) => Some(ix.clone()),
            _ => None,
        }
    }

    fn translate_router(
        &mut self,
        main_index: FuncOrExport,
        router_offset: u32,
    ) -> Result<InstructionSet, CompilerError> {
        let mut router_opcodes = InstructionSet::new();

        // find main entrypoint (it must starts with `main` keyword)
        let num_imports = self.module.imports.len_funcs as u32;

        let func_index = self.resolve_func_index(&main_index)?.unwrap_or_default();

        match main_index {
            FuncOrExport::Export(_) => {
                let call_func_type = self.module.funcs[func_index as usize];
                let func_type = self.engine.resolve_func_type(&call_func_type, Clone::clone);
                let check_idx = self.get_or_insert_check_idx(func_type);
                router_opcodes.op_i32_const(check_idx);
                // router_opcodes.op_call_internal(func_index - num_imports);
                router_opcodes.op_call_internal(func_index);
            }
            FuncOrExport::StateRouter(
                states,
                RouterInstructions {
                    state_ix: check_instr,
                    input_ix,
                    output_ix,
                },
            ) => {
                router_opcodes.extend(input_ix);
                for (state_value, state) in states.iter().enumerate() {
                    // push current and second states on the stack
                    router_opcodes.push(check_instr);
                    router_opcodes.op_i32_const(state_value);
                    // if states are not equal then skip this call
                    router_opcodes.op_i32_eq();

                    if let Some(func_index) = self.resolve_func_index(state)? {
                        router_opcodes.op_br_if_eqz(4_i32 + output_ix.len() as i32 + 1);
                        router_opcodes.op_i32_const(router_offset + router_opcodes.len() + 2 + 1);

                        let call_func_type = self.module.funcs[func_index as usize];
                        let func_type =
                            self.engine.resolve_func_type(&call_func_type, Clone::clone);
                        let check_idx = self.get_or_insert_check_idx(func_type);

                        router_opcodes.op_i32_const(check_idx);
                        router_opcodes.op_call_internal(func_index);
                        router_opcodes.extend(output_ix.clone());
                    } else if let Some(instruction) = self.resolve_global_ix(state) {
                        router_opcodes.op_br_if_eqz(2_i32 + output_ix.len() as i32 + 1);
                        router_opcodes.push(instruction);
                        router_opcodes.extend(output_ix.clone());
                    } else {
                        unreachable!("not supported router state ({:?})", state)
                    }

                    router_opcodes.op_br_indirect(0);
                }

                const INIT_PRELUDE_VALUE: i32 = 1000;

                router_opcodes.push(check_instr);
                router_opcodes.op_i32_const(INIT_PRELUDE_VALUE);
                // if states are not equal then skip this call
                router_opcodes.op_i32_eq();
                router_opcodes.op_br_if_nez(4);
                router_opcodes.op_br_indirect(0);
            }
            FuncOrExport::Func(index) => {
                let call_func_type = self.module.funcs[index as usize];
                let func_type = self.engine.resolve_func_type(&call_func_type, Clone::clone);
                let check_idx = self.get_or_insert_check_idx(func_type);
                router_opcodes.op_i32_const(check_idx);
                router_opcodes.op_call_internal(index);
            }
            FuncOrExport::Global(instruction) => {
                router_opcodes.op_local_get(1);
                router_opcodes.push(instruction);
                router_opcodes.op_local_set(2);
                router_opcodes.op_br_indirect(0);
            }
        }
        Ok(router_opcodes)
    }

    fn translate_imports_funcs(&mut self) -> Result<(), CompilerError> {
        let injection_start = self.code_section.len();

        for func_idx in 0..self.module.imports.len_funcs as u32 {
            let beginning_offset = self.code_section.len();
            self.function_beginning
                .insert(func_idx + 1, beginning_offset);

            let func_type = self.module.funcs[func_idx as usize];
            let func_type = self.engine.resolve_func_type(&func_type, Clone::clone);
            let idx = self.get_or_insert_check_idx(func_type.clone());
            let num_inputs = func_type.params();
            let num_outputs = func_type.results();

            self.code_section.op_type_check(idx);
            self.swap_stack_parameters(num_inputs.len() as u32);
            self.translate_host_call(func_idx as u32)?;
            if num_outputs.len() > 0 {
                DropKeepWithReturnParam(
                    DropKeep::new(0, num_outputs.len())
                        .map_err(|_| CompilerError::DropKeepOutOfBounds)?,
                )
                .translate(&mut self.code_section)?;
            }
            self.code_section.op_br_indirect(0);
        }

        self.injection_segments.push(Injection {
            begin: injection_start as i32,
            end: self.code_section.len() as i32,
            origin_len: 0,
        });

        Ok(())
    }

    fn translate_sections(&mut self, main_index: FuncOrExport) -> Result<(), CompilerError> {
        // lets reserve 0 index and offset for sections init
        assert_eq!(self.code_section.len(), 0, "code section must be empty");
        self.function_beginning.insert(0, 0);
        // translate global section (replaces with set/get global opcodes)
        let total_globals = self.module.globals.len();
        for i in 0..total_globals {
            self.translate_global(i as u32)?;
        }
        // translate table section (replace with grow/set table opcodes)
        self.translate_table()?;

        // translate memory section (replace with grow/load memory opcodes)
        self.translate_memory()?;
        self.translate_data()?;
        // translate router into separate instruction set
        let router_opcodes = self.translate_router(main_index, self.code_section.len() + 1)?;
        // inject main function call with return
        let return_offset = self.code_section.len() + router_opcodes.len() + 1;
        self.code_section.op_i32_const(return_offset);
        self.code_section.extend(router_opcodes);
        self.code_section.op_return();
        self.code_section.op_unreachable();
        // remember that this is injected and shifts br/br_if offset
        self.injection_segments.push(Injection {
            begin: 0,
            end: self.code_section.len() as i32,
            origin_len: 0,
        });
        Ok(())
    }

    fn translate_sections_with_state(
        &mut self,
        main_index: FuncOrExport,
    ) -> Result<(), CompilerError> {
        // lets reserve 0 index and offset for sections init
        assert_eq!(self.code_section.len(), 0, "code section must be empty");
        self.function_beginning.insert(0, 0);
        let router_opcodes = self.translate_router(main_index, self.code_section.len() + 1)?;

        let return_offset = self.code_section.len() + router_opcodes.len() + 1;
        self.code_section.op_i32_const(return_offset);
        self.code_section.extend(router_opcodes);
        self.code_section.op_return();
        self.code_section.op_unreachable();

        // translate global section (replaces with set/get global opcodes)
        let total_globals = self.module.globals.len();
        for i in 0..total_globals {
            self.translate_global(i as u32)?;
        }
        // translate table section (replace with grow/set table opcodes)
        self.translate_table()?;
        // translate memory section (replace with grow/load memory opcodes)
        self.translate_memory()?;
        self.translate_data()?;
        // translate router into separate instruction set
        // inject main function call with return
        self.code_section.op_br_indirect(0);

        // remember that this is injected and shifts br/br_if offset
        self.injection_segments.push(Injection {
            begin: 0,
            end: self.code_section.len() as i32,
            origin_len: 0,
        });

        Ok(())
    }

    fn read_memory_segment(
        memory: &DataSegment,
    ) -> Result<(UntypedValue, &[u8], bool), CompilerError> {
        match memory.kind() {
            DataSegmentKind::Active(seg) => {
                if let Some(data_offset) = seg.offset().eval_const() {
                    if seg.memory_index().into_u32() != 0 {
                        return Err(CompilerError::NotSupported("not zero index"));
                    }
                    Ok((data_offset, memory.bytes(), true))
                } else if let Some(data_offset) = seg.offset().eval_with_context(
                    |index| Value::F32(F32::from(666)),
                    |index| FuncRef::default(),
                ) {
                    #[cfg(feature = "e2e")]
                    return Ok((data_offset, memory.bytes(), true));

                    #[cfg(not(feature = "e2e"))]
                    return Err(CompilerError::NotSupported("can't eval offset"));
                } else {
                    Err(CompilerError::NotSupported("can't eval offset"))
                }
            }
            DataSegmentKind::Passive => Ok((0.into(), memory.bytes(), false)),
        }
    }

    fn translate_memory(&mut self) -> Result<(), CompilerError> {
        #[cfg(not(feature = "e2e"))]
        {
            for memory in self.module.memories.iter() {
                self.code_section
                    .add_memory_pages(memory.initial_pages().into_inner());
            }
        }

        #[cfg(feature = "e2e")]
        {
            if self.module.imports.len_memories != 0 {
                self.code_section.add_memory_pages(1);
            } else {
                for memory in self.module.memories.iter() {
                    self.code_section
                        .add_memory_pages(memory.initial_pages().into_inner());
                }
            }
        }

        Ok(())
    }

    fn translate_data(&mut self) -> Result<(), CompilerError> {
        for (idx, memory) in self.module.data_segments.iter().enumerate() {
            if let Ok((offset, bytes, is_active)) = Self::read_memory_segment(memory) {
                if is_active {
                    self.code_section.add_memory(
                        offset.with_type(ValueType::I32).i32().unwrap(),
                        bytes,
                        idx,
                    );
                } else {
                    self.code_section.add_data(bytes, idx);
                }
            }
        }
        Ok(())
    }

    fn translate_global(&mut self, global_index: u32) -> Result<(), CompilerError> {
        let len_imported = self.module.imports.len_globals;

        let globals = &self.module.globals;
        assert!(global_index < globals.len() as u32);

        if global_index < len_imported as u32 {
            self.code_section
                .op_call(self.global_start_index + global_index);
        } else {
            let global_inits = &self.module.globals_init;
            assert!(global_index as usize - len_imported < global_inits.len());

            let global_expr = &global_inits[global_index as usize - len_imported];
            if let Some(value) = global_expr.eval_const() {
                self.code_section.op_i64_const(value);
            } else if let Some(value) = global_expr.funcref() {
                self.code_section.op_ref_func(value.into_u32());
            } else if let Some(index) = global_expr.global() {
                self.code_section.op_global_get(index.into_u32());
            } else if let Some(value) = global_expr.eval_with_context(
                |index| Value::F32(F32::from(666)),
                |index| FuncRef::default(),
            ) {
                #[cfg(feature = "e2e")]
                self.code_section.op_i64_const(value.to_bits());
                #[cfg(not(feature = "e2e"))]
                self.code_section.op_unreachable();
            }
        }

        self.code_section.op_global_set(global_index);
        Ok(())
    }

    fn translate_const_expr(&self, const_expr: &ConstExpr) -> Result<UntypedValue, CompilerError> {
        #[cfg(not(feature = "e2e"))]
        {
            let init_value = const_expr.eval_const().ok_or(CompilerError::NotSupported(
                "only static global variables supported",
            ))?;

            return Ok(init_value);
        }

        #[cfg(feature = "e2e")]
        {
            let init_value = const_expr
                .eval_with_context(|index| Value::I32(666), |index| FuncRef::default())
                .ok_or(CompilerError::NotSupported(
                    "only static global variables supported",
                ))?;

            return Ok(init_value);
        }
    }

    fn translate_table(&mut self) -> Result<(), CompilerError> {
        for (table_index, table) in self.module.tables.iter().enumerate() {
            // don't use ref_func here due to the entrypoint section
            self.code_section.op_i32_const(0);
            if table_index < self.module.imports.len_tables {
                #[cfg(not(feature = "e2e"))]
                self.code_section.op_i64_const(table.minimum() as usize);
                #[cfg(feature = "e2e")]
                self.code_section.op_i64_const(10);
            } else {
                self.code_section.op_i64_const(table.minimum() as usize);
            }
            self.code_section.op_table_grow(table_index as u32);
            self.code_section.op_drop();
        }

        for (i, e) in self.module.element_segments.iter().enumerate() {
            if e.ty != ValueType::FuncRef {
                return Err(CompilerError::NotSupported(
                    "only funcref type is supported for element segments",
                ));
            }
            match &e.kind {
                ElementSegmentKind::Declared => return Ok(()),
                ElementSegmentKind::Passive => {
                    for (_, item) in e.items.items().iter().enumerate() {
                        if let Some(value) = item.funcref() {
                            self.code_section.op_i32_const(value.into_u32());
                            self.code_section.op_elem_store(i as u32);
                        }
                    }
                }
                ElementSegmentKind::Active(aes) => {
                    let dest_offset = self.translate_const_expr(aes.offset())?;
                    for (index, item) in e.items.items().iter().enumerate() {
                        self.code_section
                            .op_i32_const(dest_offset.as_u32() + index as u32);
                        if let Some(value) = item.eval_const() {
                            self.code_section.op_i64_const(value);
                        } else if let Some(value) = item.funcref() {
                            self.code_section.op_ref_func(value.into_u32());
                        }
                        self.code_section.op_table_set(aes.table_index().into_u32());
                    }
                    #[cfg(feature = "e2e")]
                    {
                        self.code_section.op_i64_const(dest_offset);
                        self.code_section.op_i64_const(0);
                        self.code_section.op_i64_const(0);
                        self.code_section.op_table_init(i as u32);
                        self.code_section.op_table_get(aes.table_index().into_u32());
                    }
                }
                _ => {}
            };
        }
        Ok(())
    }

    fn swap_stack_parameters(&mut self, param_num: u32) {
        for i in (0..param_num).rev() {
            let depth = i + 1;
            self.code_section.op_local_get(depth + 1);
            self.code_section.op_local_get(2);
            self.code_section.op_local_set(depth + 2);
            self.code_section.op_local_set(1);
        }
    }

    fn swap_target(&mut self, param_num: u32) {
        if param_num == 0 {
            return;
        }
        self.code_section.op_local_get(param_num + 1);
        for i in (0..=param_num).rev() {
            if i != 0 {
                self.code_section.op_local_get(i + 1);
                self.code_section.op_local_set(i + 2);
            } else {
                self.code_section.op_local_set(i + 1);
            }
        }
    }

    fn get_or_insert_check_idx(&mut self, func_type: FuncType) -> u32 {
        let idx = self
            .func_type_check_idx
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(idx, fn_type)| fn_type.eq(&func_type).then_some(idx));
        if let Some(idx) = idx {
            idx as u32
        } else {
            self.func_type_check_idx.borrow_mut().push(func_type);
            self.func_type_check_idx.borrow().len() as u32 - 1
        }
    }

    fn translate_function(&mut self, fn_index: u32) -> Result<(), CompilerError> {
        let import_len = self.module.imports.len_funcs;
        // don't translate import functions because we can't translate them
        if fn_index < import_len as u32 {
            return Ok(());
        }
        let dedup_func_type = self.module.funcs[fn_index as usize];

        let func_type = self
            .engine
            .resolve_func_type(&dedup_func_type, Clone::clone);
        let idx = self.get_or_insert_check_idx(func_type.clone());
        let num_inputs = func_type.params();
        let beginning_offset = self.code_section.len();

        self.code_section.op_type_check(idx);

        if !self.config.translate_func_as_inline {
            self.swap_stack_parameters(num_inputs.len() as u32);
        }

        let func_body = self
            .module
            .compiled_funcs
            .get(fn_index as usize - import_len as usize)
            .ok_or(CompilerError::MissingFunction)?;

        // reserve stack for locals
        let len_locals = self.engine.num_locals(*func_body);
        (0..len_locals).for_each(|_| {
            self.code_section.op_i32_const(0);
        });
        // translate instructions
        let (mut instr_ptr, instr_end) = self.engine.instr_ptr(*func_body);
        while instr_ptr != instr_end {
            self.translate_opcode(&mut instr_ptr, 0)?;
        }
        if !self.config.translate_func_as_inline {
            self.code_section.op_unreachable();
        }
        // remember function offset in the mapping (+1 because 0 is reserved for sections init)
        self.function_beginning
            .insert(fn_index + 1, beginning_offset);
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

    fn translate_br_table(
        &mut self,
        instr_ptr: &mut InstructionPtr,
        branch_offset: Option<BranchOffset>,
    ) -> Result<(), CompilerError> {
        if let Some(mut br_table_status) = self.br_table_status.take() {
            let drop_keep = Self::extract_drop_keep(instr_ptr);
            let injection_begin = br_table_status.instr_countdown as i32
                + br_table_status.injection_instructions.len() as i32;

            self.code_section.op_br(BranchOffset::from(injection_begin));
            self.code_section.op_return();
            br_table_status.instr_countdown -= 2;

            match branch_offset {
                Some(branch_offset) => {
                    let mut drop_keep_ixs = translate_drop_keep(drop_keep)?;

                    br_table_status
                        .injection_instructions
                        .append(&mut drop_keep_ixs);
                    br_table_status.injection_instructions.push(Instruction::Br(
                        BranchOffset::from(
                            branch_offset.to_i32() - br_table_status.instr_countdown as i32,
                        ),
                    ));
                }
                None => {
                    br_table_status
                        .injection_instructions
                        .push(Instruction::LocalGet(LocalDepth::from(
                            (drop_keep.drop() + drop_keep.keep() + 1) as u32,
                        )));

                    let mut drop_keep_ixs = translate_drop_keep(
                        DropKeep::new(drop_keep.drop() as usize + 1, drop_keep.keep() as usize + 1)
                            .map_err(|_| CompilerError::DropKeepOutOfBounds)?,
                    )?;

                    br_table_status
                        .injection_instructions
                        .append(&mut drop_keep_ixs);
                    br_table_status
                        .injection_instructions
                        .push(Instruction::BrIndirect(BranchOffset::from(0)));
                }
            }

            br_table_status
                .injection_instructions
                .push(Instruction::Return(DropKeep::none()));

            if br_table_status.instr_countdown == 0 {
                let injection_len = br_table_status.injection_instructions.len();
                for i in 0..injection_len {
                    if let Some(offset) =
                        br_table_status.injection_instructions[i].get_jump_offset()
                    {
                        br_table_status.injection_instructions[i].update_branch_offset(
                            BranchOffset::from(
                                offset.to_i32() + injection_len as i32 - i as i32 - 2,
                            ),
                        );
                    }
                }
                self.code_section
                    .instr
                    .append(&mut br_table_status.injection_instructions);
                self.br_table_status = None;
            } else {
                self.br_table_status = Some(br_table_status);
            }
        }

        Ok(())
    }

    fn translate_opcode(
        &mut self,
        instr_ptr: &mut InstructionPtr,
        _return_ptr_offset: usize,
    ) -> Result<(), CompilerError> {
        use Instruction as WI;
        let injection_begin = self.code_section.len();
        let mut opcode_count_origin = 1;

        match *instr_ptr.get() {
            WI::BrAdjust(branch_offset) => {
                opcode_count_origin += 1;
                if self.br_table_status.is_some() {
                    self.translate_br_table(instr_ptr, Some(branch_offset))?;
                } else {
                    Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                    self.code_section.op_br(branch_offset);
                    self.code_section.op_return();
                }
            }
            // WI::BrIfNez(branch_offset) => {
            //     let jump_dest = (offset as i32 + branch_offset.to_i32()) as u32;
            // }
            WI::BrAdjustIfNez(branch_offset) => {
                opcode_count_origin += 1;
                let br_if_offset = self.code_section.len();
                self.code_section.op_br_if_eqz(0);
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                let drop_keep_len = self.code_section.len() - br_if_offset + 1;
                self.code_section
                    .get_mut(br_if_offset as usize)
                    .unwrap()
                    .update_branch_offset(BranchOffset::from(1 + drop_keep_len as i32));

                // We increase break offset in negative case
                // due to jump over BrAdjustIfNez opcode injection
                let mut branch_offset = branch_offset.to_i32();
                if branch_offset < 0 {
                    branch_offset -= 3;
                }

                self.code_section.op_br(branch_offset);
                self.code_section.op_return();
            }
            WI::ReturnCallInternal(func_idx) => {
                opcode_count_origin += 1;

                let fn_index = func_idx.into_usize() as u32;

                let call_func_type = self.module.funcs[fn_index as usize];
                let func_type = self.engine.resolve_func_type(&call_func_type, Clone::clone);
                let idx = self.get_or_insert_check_idx(func_type.clone());
                let num_inputs = func_type.params();

                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;

                self.swap_target(num_inputs.len() as u32);

                self.code_section.op_i32_const(idx);
                self.code_section.op_call_internal(func_idx);
            }
            WI::ReturnCall(func) => {
                self.code_section.op_unreachable();
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                self.code_section.op_call(func);
                self.code_section.op_return();
                unreachable!("wait, should it call translate host call?");
            }
            WI::ReturnCallIndirect(sig_index) => {
                self.code_section.op_i32_const(888);
                self.code_section.op_drop();

                let drop_keep = Self::extract_drop_keep(instr_ptr);
                let call_func_type = self.module.func_types[sig_index.to_u32() as usize];
                let func_type = self.engine.resolve_func_type(&call_func_type, Clone::clone);
                let num_inputs = func_type.params();

                let table_idx = Self::extract_table(instr_ptr);
                opcode_count_origin += 2;
                // let target = self.code_section.len() + 3 + 4 + 1 + 4;

                self.code_section.op_table_get(table_idx);

                DropKeep::new(drop_keep.drop() as usize, drop_keep.keep() as usize + 1)
                    .unwrap()
                    .translate(&mut self.code_section)?;

                self.swap_target(1 + num_inputs.len() as u32);

                self.swap_stack_parameters(1);

                let idx = self.get_or_insert_check_idx(func_type.clone());
                self.code_section.op_i32_const(idx);

                self.swap_stack_parameters(1);
                self.code_section.op_br_indirect(0);
            }
            WI::Return(drop_keep) => {
                if self.br_table_status.is_some() {
                    self.translate_br_table(instr_ptr, None)?;
                } else {
                    DropKeepWithReturnParam(drop_keep).translate(&mut self.code_section)?;

                    if !self.config.translate_func_as_inline {
                        self.code_section.op_br_indirect(0);
                    }
                }
            }
            WI::ReturnIfNez(drop_keep) => {
                let br_if_offset = self.code_section.len();
                self.code_section.op_br_if_eqz(0);
                DropKeepWithReturnParam(drop_keep).translate(&mut self.code_section)?;
                let drop_keep_len = self.code_section.len() - br_if_offset;
                self.code_section
                    .get_mut(br_if_offset as usize)
                    .unwrap()
                    .update_branch_offset(BranchOffset::from(1 + drop_keep_len as i32));
                self.code_section.op_br_indirect(0);
            }
            WI::CallInternal(func_idx) => {
                let target = self.code_section.len() + 2 + 1;
                // we use this constant to remember ref func offset w/o moving function indices
                // self.function_beginning
                //     .insert(REF_FUNC_FUNCTION_OFFSET + target, target);
                // self.code_section
                //     .op_ref_func(REF_FUNC_FUNCTION_OFFSET + target - 1);
                self.code_section.op_i32_const(target);
                let fn_index = func_idx.into_usize() as u32;

                let call_func_type = self.module.funcs[fn_index as usize];
                let func_type = self.engine.resolve_func_type(&call_func_type, Clone::clone);
                let idx = self.get_or_insert_check_idx(func_type.clone());
                self.code_section.op_i32_const(idx);

                self.code_section
                    .op_call_internal(fn_index + self.module.imports.len_funcs as u32);
                // self.code_section.op_drop();
            }
            WI::CallIndirect(sig_index) => {
                let table_idx = Self::extract_table(instr_ptr);
                opcode_count_origin += 1;
                let target = self.code_section.len() + 3 + 4 + 1 + 4;

                self.code_section.op_table_get(table_idx);
                self.code_section.op_i32_const(target);
                self.swap_stack_parameters(1);

                let dedup_func_type = self.module.func_types[sig_index.to_u32() as usize];
                let func_type = self
                    .engine
                    .resolve_func_type(&dedup_func_type, Clone::clone);
                let idx = self.get_or_insert_check_idx(func_type.clone());
                self.code_section.op_i32_const(idx);

                self.swap_stack_parameters(1);
                self.code_section.op_br_indirect(0);
            }
            WI::Call(func_idx) => {
                self.translate_host_call(func_idx.to_u32())?;
            }
            WI::ConstRef(const_ref) => {
                let resolved_const = self.engine.resolve_const(const_ref).unwrap();
                self.code_section.op_i64_const(resolved_const);
            }
            WI::BrTable(target) => {
                self.br_table_status = Some(BrTableStatus {
                    injection_instructions: vec![],
                    instr_countdown: target.to_usize() as u32 * 2,
                });
                self.code_section.push(*instr_ptr.get());
            }
            WI::MemoryGrow => {
                assert!(!self.module.memories.is_empty(), "memory must be provided");
                let max_pages = self.module.memories[0]
                    .maximum_pages()
                    .unwrap_or(Pages::max())
                    .into_inner();
                self.code_section.op_local_get(1);
                self.code_section.op_memory_size();
                self.code_section.op_i32_add();
                self.code_section.op_i32_const(max_pages);
                self.code_section.op_i32_gt_s();
                self.code_section.op_br_if_eqz(4);
                self.code_section.op_drop();
                self.code_section.op_i32_const(u32::MAX);
                self.code_section.op_br(2);
                self.code_section.op_memory_grow();
            }
            WI::TableGrow(idx) => {
                let max_size = self.module.tables[idx.to_u32() as usize]
                    .maximum()
                    .unwrap_or(1024);
                self.code_section.op_local_get(1);
                self.code_section.op_table_size(idx);
                self.code_section.op_i32_add();
                self.code_section.op_i32_const(max_size);
                self.code_section.op_i32_gt_s();
                self.code_section.op_br_if_eqz(5);
                self.code_section.op_drop();
                self.code_section.op_drop();
                self.code_section.op_i32_const(u32::MAX);
                self.code_section.op_br(2);
                self.code_section.op_table_grow(idx);
            }
            // WI::LocalGet(local_depth) => {
            //     self.code_section
            //         .op_local_get(local_depth.to_usize() as u32 + 1);
            // }
            // WI::LocalSet(local_depth) => {
            //     self.code_section
            //         .op_local_set(local_depth.to_usize() as u32 + 1);
            // }
            // WI::LocalTee(local_depth) => {
            //     self.code_section
            //         .op_local_tee(local_depth.to_usize() as u32 + 1);
            // }
            _ => {
                self.code_section.push(*instr_ptr.get());
            }
        };
        let injection_end = self.code_section.len();
        if injection_end - injection_begin > opcode_count_origin as u32 {
            self.injection_segments.push(Injection {
                begin: injection_begin as i32,
                end: injection_end as i32,
                origin_len: opcode_count_origin,
            });
        }

        instr_ptr.add(1);
        Ok(())
    }

    fn resolve_host_call(&mut self, fn_index: u32) -> Result<(u32, u32), CompilerError> {
        let imports = self
            .module
            .imports
            .items
            .deref()
            .iter()
            .filter(|import| matches!(import, Imported::Func(_)))
            .collect::<Vec<_>>();
        if fn_index >= imports.len() as u32 {
            return Err(CompilerError::NotSupportedImport);
        }
        let imported = &imports[fn_index as usize];
        let import_name = match imported {
            Imported::Func(import_name) => import_name,
            _ => return Err(CompilerError::NotSupportedImport),
        };
        let import_index_and_fuel_amount = self
            .import_linker
            .ok_or(CompilerError::UnknownImport(import_name.clone()))?
            .index_mapping()
            .get(import_name)
            .ok_or(CompilerError::UnknownImport(import_name.clone()))?;
        Ok(*import_index_and_fuel_amount)
    }

    fn translate_host_call(&mut self, fn_index: u32) -> Result<(), CompilerError> {
        let (import_index, fuel_amount) = self.resolve_host_call(fn_index)?;

        if self.engine.config().get_fuel_consumption_mode().is_some() {
            self.code_section.op_consume_fuel(fuel_amount);
        }

        self.code_section.op_call(import_index);
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>, CompilerError> {
        if !self.is_translated {
            self.translate(None)?;
        }
        let bytecode = &mut self.code_section;

        let mut i = 0;
        while i < bytecode.len() as usize {
            match bytecode.instr[i] {
                Instruction::CallInternal(func) => {
                    let func_idx = func.to_u32() + 1;
                    bytecode.instr[i] = Instruction::Br(BranchOffset::from(
                        self.function_beginning[&func_idx] as i32 - i as i32,
                    ));
                }
                Instruction::Br(offset)
                | Instruction::BrIfNez(offset)
                | Instruction::BrAdjust(offset)
                | Instruction::BrAdjustIfNez(offset)
                | Instruction::BrIfEqz(offset) => {
                    let mut offset = offset.to_i32();
                    let start = i as i32;
                    let mut target = start + offset;
                    if offset > 0 {
                        for injection in &self.injection_segments {
                            if injection.begin < target && start < injection.begin {
                                offset += injection.end - injection.begin - injection.origin_len;
                                target += injection.end - injection.begin - injection.origin_len;
                            }
                        }
                    } else {
                        for injection in self.injection_segments.iter().rev() {
                            if injection.end < start && target < injection.end {
                                offset -= injection.end - injection.begin - injection.origin_len;
                                target -= injection.end - injection.begin - injection.origin_len;
                            }
                        }
                    };
                    bytecode.instr[i].update_branch_offset(BranchOffset::from(offset));
                }
                Instruction::BrTable(target) => {
                    i += target.to_usize() * 2;
                }
                _ => {}
            };
            i += 1;
        }

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

        for i in 0..bytecode.instr.len() {
            let mut code = bytecode.instr[i].clone();
            let mut affected = false;
            match code {
                Instruction::CallInternal(func) | Instruction::ReturnCallInternal(func) => {
                    let func_idx = func.to_u32() + 1;
                    let func_offset = self
                        .function_beginning
                        .get(&func_idx)
                        .ok_or(CompilerError::MissingFunction)?;
                    let state = &states[*func_offset as usize];
                    code.update_call_index(state.0);
                    affected = true;
                }
                Instruction::RefFunc(func_idx) => {
                    // if ref func refers to host call
                    let func_offset = self
                        .function_beginning
                        .get(&(func_idx.to_u32() + 1))
                        .ok_or(CompilerError::MissingFunction)?;
                    let state = &states[*func_offset as usize];
                    code.update_call_index(state.0);
                    affected = true;
                }
                Instruction::I32Const(func_idx)
                    if i < bytecode.instr.len() - 1
                        && matches!(bytecode.instr[i + 1], Instruction::ElemStore(_)) =>
                {
                    if self.function_beginning.len() > 1 || func_idx.as_u32() != 0 {
                        let func_offset = self
                            .function_beginning
                            .get(&(func_idx.as_u32() + 1))
                            .ok_or(CompilerError::MissingFunction)?;
                        code = Instruction::I32Const((*func_offset).into());
                        affected = true;
                    }
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
