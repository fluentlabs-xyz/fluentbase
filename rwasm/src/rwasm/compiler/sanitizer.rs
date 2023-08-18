use crate::{engine::bytecode::Instruction, rwasm::InstructionSet, FuncType};
use alloc::string::String;

#[derive(Default, Clone)]
pub struct Sanitizer {
    str: String,
}

impl Sanitizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_internal_fn(
        &mut self,
        func_type: &FuncType,
        instruction_set: &mut InstructionSet,
        pos: usize,
    ) {
        let mut stack_height = 0i32;
        let input_len = func_type.params().len();
        stack_height += input_len as i32;
        self.inject_opcode(instruction_set, &Instruction::F64Add, pos, stack_height)
    }

    fn inject_opcode(
        &mut self,
        instruction_set: &mut InstructionSet,
        instr: &Instruction,
        pos: usize,
        stack_height: i32,
    ) {
        self.str += format!("{instr:?}: {}\n", stack_height).as_str();
        instruction_set
            .instr
            .insert(pos, Instruction::SanitizerStackCheck(stack_height));
    }

    pub fn check_stack_height_call(
        &mut self,
        instr: &Instruction,
        _func_type: &FuncType,
        instruction_set: &mut InstructionSet,
        pos: usize,
    ) {
        let stack_height = 0i32;
        // let input_len = func_type.params().len();
        // stack_height -= input_len as i32;
        // let output_len = func_type.results().len();
        // stack_height += output_len as i32;
        self.inject_opcode(instruction_set, instr, pos + 1, stack_height)
    }

    pub fn check_stack_height(
        &mut self,
        instr: &Instruction,
        instruction_set: &mut InstructionSet,
        pos: usize,
    ) -> bool {
        let mut stack_height = 0i32;
        match *instr {
            Instruction::I32Load(_)
            | Instruction::I64Load(_)
            | Instruction::F32Load(_)
            | Instruction::F64Load(_)
            | Instruction::I32Load8S(_)
            | Instruction::I32Load8U(_)
            | Instruction::I32Load16S(_)
            | Instruction::I32Load16U(_)
            | Instruction::I64Load8S(_)
            | Instruction::I64Load8U(_)
            | Instruction::I64Load16S(_)
            | Instruction::I64Load16U(_)
            | Instruction::I64Load32S(_)
            | Instruction::I64Load32U(_) => {
                stack_height -= 1;
                stack_height += 1;
            }

            Instruction::I32Store(_)
            | Instruction::I64Store(_)
            | Instruction::F32Store(_)
            | Instruction::F64Store(_)
            | Instruction::I32Store8(_)
            | Instruction::I32Store16(_)
            | Instruction::I64Store8(_)
            | Instruction::I64Store16(_)
            | Instruction::I64Store32(_) => {
                stack_height -= 2;
            }

            Instruction::ConstRef(_) => stack_height += 1,

            Instruction::I32LtS
            | Instruction::I32LtU
            | Instruction::I32GtS
            | Instruction::I32GtU
            | Instruction::I32LeS
            | Instruction::I32LeU
            | Instruction::I32GeS
            | Instruction::I32GeU
            | Instruction::I64Eqz
            | Instruction::I64Eq
            | Instruction::I64Ne
            | Instruction::I64LtS
            | Instruction::I64LtU
            | Instruction::I64GtS
            | Instruction::I64GtU
            | Instruction::I64LeS
            | Instruction::I64LeU
            | Instruction::I64GeS
            | Instruction::I64GeU
            | Instruction::F32Eq
            | Instruction::F32Ne
            | Instruction::F32Lt
            | Instruction::F32Gt
            | Instruction::F32Le
            | Instruction::F32Ge
            | Instruction::F64Eq
            | Instruction::F64Ne
            | Instruction::F64Lt
            | Instruction::F64Gt
            | Instruction::F64Le
            | Instruction::F64Ge => {
                stack_height -= 2;
                stack_height += 1;
            }

            Instruction::I32Add
            | Instruction::I32Sub
            | Instruction::I32Mul
            | Instruction::I32DivS
            | Instruction::I32DivU
            | Instruction::I32RemS
            | Instruction::I32RemU
            | Instruction::I32And
            | Instruction::I32Or
            | Instruction::I32Xor
            | Instruction::I32Shl
            | Instruction::I32ShrS
            | Instruction::I32ShrU
            | Instruction::I32Rotl
            | Instruction::I32Rotr
            | Instruction::I64Add
            | Instruction::I64Sub
            | Instruction::I64Mul
            | Instruction::I64DivS
            | Instruction::I64DivU
            | Instruction::I64RemS
            | Instruction::I64RemU
            | Instruction::I64And
            | Instruction::I64Or
            | Instruction::I64Xor
            | Instruction::I64Shl
            | Instruction::I64ShrS
            | Instruction::I64ShrU
            | Instruction::I64Rotl
            | Instruction::I64Rotr
            | Instruction::F32Add
            | Instruction::F32Sub
            | Instruction::F32Mul
            | Instruction::F32Div
            | Instruction::F32Min
            | Instruction::F32Max
            | Instruction::F32Copysign
            | Instruction::F64Add
            | Instruction::F64Sub
            | Instruction::F64Mul
            | Instruction::F64Div
            | Instruction::F64Min
            | Instruction::F64Max
            | Instruction::F64Copysign => {
                stack_height -= 2;
                stack_height += 1;
            }

            Instruction::BrIfEqz(_)
            | Instruction::BrIfNez(_)
            | Instruction::BrAdjustIfNez(_)
            | Instruction::BrTable(_)
            | Instruction::CallIndirect(_)
            | Instruction::Drop => {
                stack_height -= 1;
            }

            Instruction::Select => {
                stack_height -= 3;
                stack_height += 1;
            }

            Instruction::RefFunc(_) | Instruction::LocalGet(_) => stack_height += 1,
            Instruction::LocalSet(_) => stack_height -= 1,
            Instruction::GlobalGet(_) => stack_height += 1,
            Instruction::GlobalSet(_) => stack_height -= 1,
            Instruction::MemorySize => stack_height += 1,
            Instruction::MemoryInit(_) | Instruction::MemoryFill | Instruction::MemoryCopy => {
                stack_height -= 3
            }
            Instruction::TableSize(_) => stack_height += 1,
            Instruction::TableGrow(_) => stack_height -= 1,
            Instruction::TableCopy(_) | Instruction::TableFill(_) => stack_height -= 3,
            Instruction::TableSet(_) => stack_height -= 2,
            Instruction::TableInit(_) => stack_height -= 3,
            Instruction::I32Const(_) | Instruction::I64Const(_) => stack_height += 1,
            _ => {}
        }
        self.inject_opcode(instruction_set, instr, pos + 1, stack_height);
        true
    }
}
