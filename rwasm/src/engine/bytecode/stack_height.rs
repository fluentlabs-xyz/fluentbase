use crate::engine::bytecode::Instruction;

#[derive(Debug, Copy, Clone)]
pub enum RwOp {
    StackWrite(u32),
    StackRead(u32),
    GlobalWrite(u32),
    GlobalRead(u32),
    MemoryWrite(u32),
    MemoryRead(u32),
    TableWrite,
    TableRead,
}

impl Instruction {
    pub fn get_rw_ops(&self) -> Vec<RwOp> {
        let mut stack_ops = Vec::new();
        match *self {
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
                stack_ops.push(RwOp::MemoryRead(val.into_inner()));
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
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::MemoryWrite(val.into_inner()));
            }

            Instruction::ConstRef(_) => stack_ops.push(RwOp::StackWrite(0)),
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
                stack_ops.push(RwOp::StackRead(0));
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
            Instruction::BrIfEqz(_)
            | Instruction::BrIfNez(_)
            | Instruction::BrAdjustIfNez(_)
            | Instruction::BrTable(_)
            | Instruction::CallIndirect(_)
            | Instruction::Drop => {
                stack_ops.push(RwOp::StackRead(0));
            }
            Instruction::Select => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::RefFunc(_) => stack_ops.push(RwOp::StackWrite(0)),
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
            Instruction::GlobalGet(val) => {
                stack_ops.push(RwOp::GlobalRead(val.to_u32()));
                stack_ops.push(RwOp::StackWrite(0));
            }
            Instruction::GlobalSet(val) => {
                stack_ops.push(RwOp::StackRead(0));
                stack_ops.push(RwOp::GlobalWrite(val.to_u32()));
            }
            Instruction::MemorySize => stack_ops.push(RwOp::StackWrite(0)),
            // Instruction::MemoryInit(_) | Instruction::MemoryFill | Instruction::MemoryCopy => {
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            //     unreachable!("more memory ops?")
            // }
            // Instruction::TableSize(_) => stack_ops.push(RwOps::StackWrite),
            // Instruction::TableGrow(_) => stack_ops.push(RwOps::StackRead),
            // Instruction::TableCopy(_) | Instruction::TableFill(_) => {
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            // }
            // Instruction::TableSet(_) => {
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            // }
            // Instruction::TableInit(_) => {
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            //     stack_ops.push(RwOps::StackRead);
            // }
            Instruction::I32Const(_) | Instruction::I64Const(_) => {
                stack_ops.push(RwOp::StackWrite(0))
            }
            _ => {}
        }
        stack_ops
    }
}
