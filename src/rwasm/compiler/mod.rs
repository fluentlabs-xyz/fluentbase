use crate::engine::bytecode::BranchOffset;
use crate::rwasm::binary_format::{BinaryFormat, BinaryFormatWriter};
use crate::{
    common::UntypedValue,
    engine::bytecode::{AddressOffset, Instruction},
    engine::DropKeep,
    module::{DataSegmentKind, Imported},
    rwasm::instruction_set::InstructionSet,
    rwasm::{WazmError, WazmResult},
    Config, Engine, Linker, Module,
};
use alloc::{collections::BTreeMap, vec::Vec};
use byteorder::{BigEndian, ByteOrder};
use core::ops::Deref;

mod drop_keep;
mod opcode;

pub trait Translator {
    fn translate_to_vec(&self) -> WazmResult<Vec<Instruction>> {
        let mut result = InstructionSet::new();
        self.translate(&mut result)?;
        Ok(result.finalize())
    }

    fn translate(&self, result: &mut InstructionSet) -> WazmResult<()>;
}

pub struct Compiler {
    engine: Engine,
    module: Module,
    linker: Linker<()>,
    // translation state
    code_section: InstructionSet,
    function_mapping: BTreeMap<u32, u32>,
    call_mapping: BTreeMap<u32, u32>,
}

impl Compiler {
    pub fn new(wasm_binary: &Vec<u8>) -> Result<Self, WazmError> {
        let mut config = Config::default();
        config.consume_fuel(false);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_binary.as_slice()).map_err(|e| WazmError::ModuleError(e))?;
        let linker = Linker::new(&engine);
        Ok(Compiler {
            engine,
            module,
            linker,
            code_section: InstructionSet::new(),
            function_mapping: BTreeMap::new(),
            call_mapping: BTreeMap::new(),
        })
    }

    pub fn translate(&mut self) -> Result<(), WazmError> {
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
            .ok_or(WazmError::MissingEntrypoint)?
            .into_func_idx()
            .ok_or(WazmError::InternalError("unresolved function index"))?;
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

    pub fn translate_wo_entrypoint(&mut self) -> Result<(), WazmError> {
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

    fn translate_memory(&mut self) -> Result<(), WazmError> {
        for memory in self.module.data_segments.iter() {
            let (offset, bytes) = match memory.kind() {
                DataSegmentKind::Active(seg) => {
                    let data_offset = seg
                        .offset()
                        .eval_const()
                        .ok_or(WazmError::NotSupportedMemory("can't eval offset"))?;
                    if seg.memory_index().into_u32() != 0 {
                        return Err(WazmError::NotSupportedMemory("not zero index"));
                    }
                    (data_offset, memory.bytes())
                }
                DataSegmentKind::Passive => {
                    return Err(WazmError::NotSupportedMemory("passive mode is not supported"));
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

    fn translate_global(&mut self, global_index: u32) -> Result<(), WazmError> {
        let len_imported = self.module.imports.len_globals;
        let globals = &self.module.globals[len_imported..];
        assert!(global_index < globals.len() as u32);
        let global_inits = &self.module.globals_init;
        assert!(global_index < global_inits.len() as u32);
        let init_value = global_inits[global_index as usize]
            .eval_const()
            .ok_or(WazmError::InternalError("only static global variables supported"))?;
        self.code_section
            .push(Instruction::I64Const(UntypedValue::from(init_value.to_bits())));
        self.code_section.push(Instruction::GlobalSet((global_index).into()));
        Ok(())
    }

    fn translate_function(&mut self, fn_index: u32) -> Result<(), WazmError> {
        let import_len = self.module.imports.len_funcs;
        // don't translate import functions because we can't translate them
        if fn_index < import_len as u32 {
            return Ok(());
        }
        let func_body = self
            .module
            .compiled_funcs
            .get(fn_index as usize - import_len)
            .ok_or(WazmError::MissingFunction)?;
        let instructions = self.engine.instr_vec(*func_body);
        // translate instructions
        let beginning_offset = self.code_section.len();
        for instr in instructions.iter() {
            self.translate_opcode(instr)?;
        }
        // remember function offset in the mapping
        self.function_mapping.insert(fn_index, beginning_offset);
        Ok(())
    }

    pub(crate) fn translate_control_flow(&mut self, instr: &Instruction) -> Result<(), WazmError> {
        use Instruction as WI;
        let mut opcodes = InstructionSet::new();
        match *instr {
            // WI::Br(branch_params) => {
            // let drop_keep_opcodes = branch_params.drop_keep().translate_to_vec()?;
            // opcodes.extend(drop_keep_opcodes);
            // opcodes.op_br(branch_params.offset().into_i32());
            // }
            // WI::BrIfEqz(branch_params) => {
            // let drop_keep_opcodes = branch_params.drop_keep().translate_to_vec()?;
            // opcodes.op_br_if_nez(1 + drop_keep_opcodes.len() as i32);
            // opcodes.extend(drop_keep_opcodes);
            // opcodes.op_br(branch_params.offset().into_i32());
            // }
            // WI::BrIfNez(branch_params) => {
            // let drop_keep_opcodes = branch_params.drop_keep().translate_to_vec()?;
            // opcodes.op_br_if_eqz(1 + drop_keep_opcodes.len() as i32);
            // opcodes.extend(drop_keep_opcodes);
            // opcodes.op_br(branch_params.offset().into_i32());
            // }
            // WI::BrTable(len_targets) => {
            //     opcodes.op_br_table(len_targets.into_inner());
            // }
            // WI::Return(drop_keep) => {
            // lets keep return offset on the stack
            // if drop_keep.drop() > 0 || drop_keep.keep() > 0 {
            //     let drop_keep_opcodes = drop_keep.translate_to_vec()?;
            //     opcodes.extend(drop_keep_opcodes);
            //     opcodes.op_return(DropKeep::none());
            // } else {
            //     opcodes.op_return(DropKeep::none());
            // }
            // }
            // WI::ReturnIfNez(drop_keep) => {
            //     let drop_keep_opcodes = drop_keep.translate_to_vec()?;
            //     opcodes.op_br_if_eqz(1 + drop_keep_opcodes.len() as i32);
            //     opcodes.extend(drop_keep_opcodes);
            //     opcodes.op_return(DropKeep::none());
            // }
            // WI::ReturnCall(func) => {
            // let drop_keep_opcodes = drop_keep.translate_to_vec()?;
            // opcodes.extend(drop_keep_opcodes);
            // return self.translate_call(&instr, func.into_inner());
            // }
            // WI::ReturnCallIndirect(sig) => {
            // drop_keep.keep += 1;
            // let drop_keep_opcodes = drop_keep.translate_to_vec()?;
            // opcodes.extend(drop_keep_opcodes);
            // opcodes.op_return_call_indirect(table.into_inner(), DropKeep::none());
            // }
            // WI::Call(func_idx) => return self.translate_call(&instr, func_idx.into_inner()),
            // WI::CallIndirect(table) => {
            //     opcodes.op_call_indirect(table.into_inner());
            // }
            _ => unreachable!("don't route here with this opcode: {:?}", instr),
        }
        self.code_section.extend(opcodes);
        Ok(())
    }

    fn translate_opcode(&mut self, instr: &Instruction) -> Result<(), WazmError> {
        use Instruction as WI;
        match instr {
            WI::Br(_) | WI::BrIfEqz(_) | WI::BrIfNez(_) | WI::BrTable { .. } => {
                return self.translate_control_flow(instr)
            }
            WI::Return(_) | WI::ReturnIfNez(_) | WI::ReturnCall { .. } | WI::ReturnCallIndirect { .. } => {
                return self.translate_control_flow(instr)
            }
            WI::Call(_) | WI::CallIndirect { .. } => {
                return self.translate_control_flow(instr);
            }
            _ => {
                instr.translate(&mut self.code_section)?;
            }
        };
        Ok(())
    }

    fn translate_call(&mut self, instr: &Instruction, fn_index: u32) -> Result<(), WazmError> {
        // for basic functions jump remember function index
        let import_len = self.module.imports.len_funcs;
        if fn_index >= import_len as u32 {
            // lets store call for function call to remember it's position (we don't know offset right now)
            match instr {
                Instruction::Call(_) => {
                    // self.code_section.op_call(0);
                }
                Instruction::ReturnCall { .. } => {
                    // self.code_section.op_br(0);
                }
                _ => unreachable!("unknown call instruction: {:?}", instr),
            }
            // remember opcode offset for the function index to fix break jump offset later
            let opcode_offset = self.code_section.len() as u32;
            self.call_mapping.insert(opcode_offset - 1, fn_index);
            return Ok(());
        }
        // special case for imported methods
        // let imports = self.module.imports.items.deref();
        // if fn_index >= imports.len() as u32 {
        //     return Err(WazmError::NotSupportedImport);
        // }
        // let imported = &imports[fn_index as usize];
        // let import_name = match imported {
        //     Imported::Func(import_name) => import_name,
        //     _ => return Err(WazmError::NotSupportedImport),
        // };
        // let import_code = resolve_host_call(import_name.module.deref(), import_name.field.deref())?;
        // self.code_section.push(Instruction::Call(import_code));
        unreachable!("not supported yet");
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>, WazmError> {
        let bytecode = self.code_section.clone();

        let mut states: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        let mut buffer_offset = 0u32;
        for code in bytecode.0.iter() {
            let mut buffer: [u8; 100] = [0; 100];
            let mut binary_writer = BinaryFormatWriter::new(&mut buffer[..]);
            code.write_binary(&mut binary_writer)
                .map_err(|e| WazmError::BinaryFormat(e))?;
            let buffer = binary_writer.to_vec();
            let buffer_size = buffer.len() as u32;
            states.push((buffer_offset, buffer_size, buffer));
            buffer_offset += buffer_size;
        }

        for (i, code) in bytecode.0.iter().enumerate() {
            if let Some(jump_offset) = code.get_jump_offset() {
                let jump_label = if let Some(fn_index) = self.call_mapping.get(&(i as u32)) {
                    *self.function_mapping.get(fn_index).ok_or(WazmError::MissingFunction)? as i32
                } else {
                    jump_offset.to_i32() + i as i32
                } as usize;
                let target_state = states.get(jump_label).ok_or(WazmError::OutOfBuffer)?;
                let mut code = code.clone();
                code.update_branch_offset(BranchOffset::from(target_state.0 as i32));
                let current_state = states.get_mut(i).ok_or(WazmError::OutOfBuffer)?;
                current_state.2.clear();
                code.write_binary_to_vec(&mut current_state.2)
                    .map_err(|e| WazmError::BinaryFormat(e))?;
            }
        }

        let res = states.iter().fold(Vec::new(), |mut res, state| {
            res.extend(&state.2);
            res
        });
        Ok(res)
    }
}
