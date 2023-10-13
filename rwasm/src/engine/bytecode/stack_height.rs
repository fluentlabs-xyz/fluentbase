use crate::engine::bytecode::Instruction;
use alloc::vec::Vec;

#[derive(Debug, Copy, Clone)]
pub enum RwTableOp {
    ElemRead(u32, u32),
    ElemWrite(u32, u32),
    SizeRead(u32),
    SizeWrite(u32),
}

#[derive(Debug, Copy, Clone)]
pub enum RwOp {
    StackWrite(u32),
    StackRead(u32),
    GlobalWrite(u32),
    GlobalRead(u32),
    MemoryWrite {
        offset: u32,
        length: u32,
        signed: bool,
    },
    MemoryRead {
        offset: u32,
        length: u32,
        signed: bool,
    },
    MemorySizeWrite,
    MemorySizeRead,
    TableSizeRead(u32),
    TableSizeWrite(u32),
    TableElemRead(u32),
    TableElemWrite(u32),
}

impl Instruction {
    pub fn get_rw_count(&self) -> usize {
        let mut rw_count = 0;
        for rw_op in self.get_rw_ops() {
            match rw_op {
                RwOp::MemoryWrite { length, .. } => rw_count += length as usize,
                RwOp::MemoryRead { length, .. } => rw_count += length as usize,
                _ => rw_count += 1,
            }
        }
        rw_count
    }

    pub fn get_rw_ops(&self) -> Vec<RwOp> {
        let mut stack_ops = Vec::new();
        match *self {
            Instruction::LocalGet(local_depth) => {
                stack_ops.push(RwOp::StackRead(local_depth.to_usize() as u32));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::LocalSet(local_depth) => {
                stack_ops.push(RwOp::StackRead(0));
                // local depth can't be zero otherwise this op is useless
                if local_depth.to_usize() > 0 {
                    stack_ops.push(RwOp::StackWrite(local_depth.to_usize() as u32 - 1));
                } else {
                    stack_ops.push(RwOp::StackWrite(0));
                }
            }
            Instruction::LocalTee(local_depth) => {
                stack_ops.push(RwOp::StackRead(0));
                // local depth can't be zero otherwise this op is useless
                if local_depth.to_usize() > 0 {
                    stack_ops.push(RwOp::StackWrite(local_depth.to_usize() as u32 - 1));
                } else {
                    stack_ops.push(RwOp::StackWrite(0));
                }
            }
            Instruction::Br(_) => {}
            Instruction::BrIfEqz(_) | Instruction::BrIfNez(_) => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::BrAdjust(_) => {}
            Instruction::BrAdjustIfNez(_) | Instruction::BrTable(_) => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::Unreachable | Instruction::ConsumeFuel(_) | Instruction::Return(_) => {}
            Instruction::ReturnIfNez(_) => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::ReturnCallInternal(_) | Instruction::ReturnCall(_) => {}
            Instruction::ReturnCallIndirect(_) => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::CallInternal(_) => {}
            Instruction::Call(_) => {}
            Instruction::CallIndirect(_) => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::Drop => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::Select => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::GlobalGet(val) => {
                stack_ops.push(RwOp::GlobalRead(val.to_u32()));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::GlobalSet(val) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::GlobalWrite(val.to_u32()));
            }
            Instruction::I32Load(val)
            | Instruction::I64Load(val)
            | Instruction::F32Load(val)
            | Instruction::F64Load(val)
            | Instruction::I32Load8S(val)
            | Instruction::I32Load8U(val)
            | Instruction::I32Load16S(val)
            | Instruction::I32Load16U(val)
            | Instruction::I64Load8S(val)
            | Instruction::I64Load8U(val)
            | Instruction::I64Load16S(val)
            | Instruction::I64Load16U(val)
            | Instruction::I64Load32S(val)
            | Instruction::I64Load32U(val) => {
                let (_, commit_byte_len, signed) = Self::load_instr_meta(self);
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::MemoryRead {
                    offset: val.into_inner(),
                    length: commit_byte_len as u32,
                    signed,
                });
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::I32Store(val)
            | Instruction::I64Store(val)
            | Instruction::F32Store(val)
            | Instruction::F64Store(val)
            | Instruction::I32Store8(val)
            | Instruction::I32Store16(val)
            | Instruction::I64Store8(val)
            | Instruction::I64Store16(val)
            | Instruction::I64Store32(val) => {
                let length = Self::store_instr_meta(self);
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::MemoryWrite {
                    offset: val.into_inner(),
                    length: length as u32,
                    signed: false,
                });
            }
            Instruction::MemorySize => {
                stack_ops.push(RwOp::MemorySizeRead);
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::MemoryGrow => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
                stack_ops.push(RwOp::MemorySizeWrite);
            }
            Instruction::MemoryFill | Instruction::MemoryCopy => {
                // unreachable!("not implemented here")
            }
            Instruction::MemoryInit(_) => {}
            Instruction::DataDrop(_) => {}

            Instruction::TableSize(table_idx) => {
                stack_ops.push(RwOp::TableSizeRead(table_idx.to_u32()));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::TableGrow(table_idx) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::TableSizeWrite(table_idx.to_u32()));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::TableFill(_) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::TableGet(_table_idx) => {
                stack_ops.push(RwOp::StackRead(0));
                //stack_ops.push(RwOp::TableElemRead(table_idx.to_u32()));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::TableSet(table_idx) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                //stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::TableElemWrite(table_idx.to_u32()));
                //stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::TableCopy(_) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::TableInit(_) => {}

            Instruction::ElemDrop(_) => {}
            Instruction::RefFunc(_) => {
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::I32Const(_) | Instruction::I64Const(_) => {
                stack_ops.push(RwOp::StackWrite(0))
            }
            Instruction::ConstRef(_) => stack_ops.push(RwOp::StackWrite(0)),

            Instruction::I32Eqz
            | Instruction::I32Eq
            | Instruction::I64Eqz
            | Instruction::I64Eq
            | Instruction::I32Ne
            | Instruction::I64Ne => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::I32LtS
            | Instruction::I32LtU
            | Instruction::I32GtS
            | Instruction::I32GtU
            | Instruction::I32LeS
            | Instruction::I32LeU
            | Instruction::I32GeS
            | Instruction::I32GeU
            | Instruction::I32Eq
            | Instruction::I32Ne
            | Instruction::I64LtS
            | Instruction::I64LtU
            | Instruction::I64GtS
            | Instruction::I64GtU
            | Instruction::I64LeS
            | Instruction::I64LeU
            | Instruction::I64GeS
            | Instruction::I64GeU
            | Instruction::F32Eq
            | Instruction::F32Lt
            | Instruction::F32Gt
            | Instruction::F32Le
            | Instruction::F32Ge
            | Instruction::F32Ne
            | Instruction::F64Eq
            | Instruction::F64Ne
            | Instruction::F64Lt
            | Instruction::F64Gt
            | Instruction::F64Le
            | Instruction::F64Ge => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }

            Instruction::I32Clz | Instruction::I32Ctz | Instruction::I32Popcnt => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
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
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }

            Instruction::I32WrapI64
            | Instruction::I32TruncF32S
            | Instruction::I32TruncF32U
            | Instruction::I32TruncF64S
            | Instruction::I32TruncF64U
            | Instruction::I64ExtendI32S
            | Instruction::I64ExtendI32U
            | Instruction::I64TruncF32S
            | Instruction::I64TruncF32U
            | Instruction::I64TruncF64S
            | Instruction::I64TruncF64U
            | Instruction::F32ConvertI32S
            | Instruction::F32ConvertI32U
            | Instruction::F32ConvertI64S
            | Instruction::F32ConvertI64U
            | Instruction::F32DemoteF64
            | Instruction::F64ConvertI32S
            | Instruction::F64ConvertI32U
            | Instruction::F64ConvertI64S
            | Instruction::F64ConvertI64U
            | Instruction::F64PromoteF32
            | Instruction::I32Extend8S
            | Instruction::I32Extend16S
            | Instruction::I64Extend8S
            | Instruction::I64Extend16S
            | Instruction::I64Extend32S
            | Instruction::I32TruncSatF32S
            | Instruction::I32TruncSatF32U
            | Instruction::I32TruncSatF64S
            | Instruction::I32TruncSatF64U
            | Instruction::I64TruncSatF32S
            | Instruction::I64TruncSatF32U
            | Instruction::I64TruncSatF64S
            | Instruction::I64TruncSatF64U => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }

            Instruction::I32Ctz
            | Instruction::I64Ctz
            | Instruction::I32Clz
            | Instruction::I64Clz
            | Instruction::I32Popcnt
            | Instruction::I64Popcnt => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }

            _ => unreachable!("not supported rws for opcode: {:?}", self),
        }
        stack_ops
    }

    pub fn get_stack_diff(&self) -> i32 {
        let mut stack_diff = 0;
        for rw_op in self.get_rw_ops() {
            match rw_op {
                RwOp::StackWrite(_) => stack_diff += 1,
                RwOp::StackRead(_) => stack_diff -= 1,
                _ => {}
            }
        }
        stack_diff
    }
}
