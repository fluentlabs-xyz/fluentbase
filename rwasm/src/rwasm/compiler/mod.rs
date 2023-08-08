use crate::{
    arena::ArenaIndex,
    common::UntypedValue,
    engine::{
        bytecode::{AddressOffset, BranchOffset, FuncIdx, Instruction},
        code_map::InstructionPtr,
        DropKeep,
    },
    module::{DataSegmentKind, ImportName, Imported},
    rwasm::{
        binary_format::{BinaryFormat, BinaryFormatError, BinaryFormatWriter},
        instruction_set::InstructionSet,
    },
    Config,
    Engine,
    Module,
};
use alloc::{collections::BTreeMap, vec::Vec};
use byteorder::{BigEndian, ByteOrder};
use core::ops::Deref;

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
}

pub trait Translator {
    fn translate(&self, result: &mut InstructionSet) -> Result<(), CompilerError>;
}

pub struct Compiler {
    engine: Engine,
    module: Module,
    // translation state
    pub(crate) code_section: InstructionSet,
    function_mapping: BTreeMap<u32, u32>,
    host_function_mapping: BTreeMap<ImportName, u32>,
    is_translated: bool,
}

impl Compiler {
    pub fn new(wasm_binary: &Vec<u8>) -> Result<Self, CompilerError> {
        Self::new_with_linker(wasm_binary, BTreeMap::new())
    }

    pub fn new_with_linker(
        wasm_binary: &Vec<u8>,
        host_function_mapping: BTreeMap<ImportName, u32>,
    ) -> Result<Self, CompilerError> {
        let mut config = Config::default();
        config.consume_fuel(false);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_binary.as_slice()).map_err(|e| CompilerError::ModuleError(e))?;
        Ok(Compiler {
            engine,
            module,
            code_section: InstructionSet::new(),
            function_mapping: BTreeMap::new(),
            host_function_mapping,
            is_translated: false,
        })
    }

    pub fn translate(&mut self) -> Result<(), CompilerError> {
        if self.is_translated {
            unreachable!("already translated");
        }
        // translate memory and global first
        let total_globals = self.module.globals.len();
        for i in 0..total_globals {
            self.translate_global(i as u32)?;
        }
        self.translate_memory()?;
        // find main entrypoint (it must starts with `main` keyword)
        let main_index = self
            .module
            .exports
            .get("main")
            .ok_or(CompilerError::MissingEntrypoint)?
            .into_func_idx()
            .ok_or(CompilerError::MissingEntrypoint)?;
        // translate main entrypoint
        self.translate_function(main_index)?;
        // translate rest functions
        let total_fns = self.module.funcs.len();
        for i in 0..total_fns {
            if i != main_index as usize {
                self.translate_function(i as u32)?;
            }
        }
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
        Ok(())
    }

    fn translate_memory(&mut self) -> Result<(), CompilerError> {
        for memory in self.module.data_segments.iter() {
            let (offset, bytes) = match memory.kind() {
                DataSegmentKind::Active(seg) => {
                    let data_offset = seg
                        .offset()
                        .eval_const()
                        .ok_or(CompilerError::NotSupported("can't eval offset"))?;
                    if seg.memory_index().into_u32() != 0 {
                        return Err(CompilerError::NotSupported("not zero index"));
                    }
                    (data_offset, memory.bytes())
                }
                DataSegmentKind::Passive => {
                    return Err(CompilerError::NotSupported("passive mode is not supported"));
                }
            };
            let mut offset = offset.to_bits() as u32;
            for chunk in bytes.chunks(8) {
                let (opcode, value) = match chunk.len() {
                    8 => (
                        Instruction::I64Store(AddressOffset::from(0)),
                        BigEndian::read_u64(chunk),
                    ),
                    4 => (
                        Instruction::I64Store32(AddressOffset::from(0)),
                        BigEndian::read_u32(chunk) as u64,
                    ),
                    2 => (
                        Instruction::I32Store16(AddressOffset::from(0)),
                        BigEndian::read_u16(chunk) as u64,
                    ),
                    1 => (Instruction::I32Store8(AddressOffset::from(0)), chunk[0] as u64),
                    _ => {
                        unreachable!("not possible chunk len: {}", chunk.len())
                    }
                };
                self.code_section.op_i32_const(offset);
                self.code_section.op_i64_const(value);
                self.code_section.push(opcode);
                offset += chunk.len() as u32;
            }
        }
        Ok(())
    }

    fn translate_global(&mut self, global_index: u32) -> Result<(), CompilerError> {
        let len_imported = self.module.imports.len_globals;
        let globals = &self.module.globals[len_imported..];
        assert!(global_index < globals.len() as u32);
        let global_inits = &self.module.globals_init;
        assert!(global_index < global_inits.len() as u32);
        let init_value = global_inits[global_index as usize]
            .eval_const()
            .ok_or(CompilerError::NotSupported("only static global variables supported"))?;
        self.code_section
            .push(Instruction::I64Const(UntypedValue::from(init_value.to_bits())));
        self.code_section.push(Instruction::GlobalSet((global_index).into()));
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

    fn translate_opcode(&mut self, instr_ptr: &mut InstructionPtr) -> Result<(), CompilerError> {
        use Instruction as WI;
        match *instr_ptr.get() {
            WI::BrAdjust(branch_offset) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                self.code_section.op_br(branch_offset);
                self.code_section.op_return(DropKeep::none());
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
                self.code_section.op_return(DropKeep::none());
            }
            WI::ReturnCallInternal(func) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                let fn_index = func.into_usize() as u32;
                self.code_section.op_return_call_internal(fn_index);
                self.code_section.op_return(DropKeep::none());
            }
            WI::ReturnCall(func) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                self.code_section.op_return_call(func);
                self.code_section.op_return(DropKeep::none());
            }
            WI::ReturnCallIndirect(sig) => {
                Self::extract_drop_keep(instr_ptr).translate(&mut self.code_section)?;
                self.code_section.op_return_call_indirect(sig);
                self.code_section.op_return(DropKeep::none());
            }
            WI::Return(drop_keep) => {
                drop_keep.translate(&mut self.code_section)?;
                self.code_section.op_return(DropKeep::none());
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
                self.code_section.op_return_if_nez(DropKeep::none());
            }
            WI::CallInternal(func_idx) => {
                let fn_index = func_idx.into_usize() as u32;
                self.code_section.op_call_internal(fn_index);
            }
            WI::Call(func_idx) => {
                self.translate_host_call(func_idx.to_u32())?;
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
            .host_function_mapping
            .get(import_name)
            .ok_or(CompilerError::UnknownImport(import_name.clone()))?;
        self.code_section.op_call(*import_index);
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>, CompilerError> {
        if !self.is_translated {
            self.translate()?;
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
