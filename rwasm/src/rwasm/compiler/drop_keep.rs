use crate::{
    engine::{
        bytecode::{Instruction, LocalDepth},
        DropKeep,
    },
    rwasm::{
        compiler::{CompilerError, Translator},
        instruction_set::InstructionSet,
    },
};
use alloc::vec::Vec;

pub(crate) fn translate_drop_keep(drop_keep: DropKeep) -> Result<Vec<Instruction>, CompilerError> {
    let mut result = Vec::new();
    let (drop, keep) = (drop_keep.drop(), drop_keep.keep());
    if drop == 0 {
        return Ok(result);
    }
    if drop >= keep {
        (0..keep).for_each(|_| result.push(Instruction::LocalSet(LocalDepth::from(drop as u32))));
        (0..(drop - keep)).for_each(|_| result.push(Instruction::Drop));
    } else {
        (0..keep).for_each(|i| {
            result.push(Instruction::LocalGet(LocalDepth::from(
                keep as u32 - i as u32,
            )));
            result.push(Instruction::LocalSet(LocalDepth::from(
                keep as u32 + drop as u32 - i as u32,
            )));
        });
        (0..drop).for_each(|_| result.push(Instruction::Drop));
    }
    Ok(result)
}

impl Translator for DropKeep {
    fn translate(&self, result: &mut InstructionSet) -> Result<(), CompilerError> {
        let drop_keep_opcodes = translate_drop_keep(*self)?;
        result.instr.extend(&drop_keep_opcodes);
        Ok(())
    }
}

#[derive(Debug)]
pub struct DropKeepWithReturnParam(pub DropKeep);

impl Translator for DropKeepWithReturnParam {
    fn translate(&self, result: &mut InstructionSet) -> Result<(), CompilerError> {
        if self.0.drop() == 0 && self.0.keep() == 0 {
            return Ok(());
        }
        result.op_local_get((self.0.drop() + self.0.keep() + 1) as u32);
        let drop_keep_opcodes = translate_drop_keep(
            DropKeep::new(self.0.drop() as usize + 1, self.0.keep() as usize + 1)
                .map_err(|_| CompilerError::DropKeepOutOfBounds)?,
        )?;
        result.instr.extend(&drop_keep_opcodes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        engine::{bytecode::Instruction, DropKeep},
        rwasm::compiler::drop_keep::translate_drop_keep,
    };

    #[test]
    fn test_drop_keep_translation() {
        macro_rules! drop_keep {
            ($drop:literal, $keep:literal) => {
                DropKeep::new($drop, $keep).unwrap()
            };
        }
        let tests = vec![
            (vec![1, 2], vec![1, 2], drop_keep!(0, 0)),
            (vec![1, 2, 3], vec![1, 2, 3], drop_keep!(0, 3)),
            (vec![1, 2, 3, 4], vec![3, 4], drop_keep!(2, 2)),
            (vec![2, 3, 7], vec![3, 7], drop_keep!(1, 2)),
            (vec![1, 2, 3, 4, 5, 6], vec![3, 4, 5, 6], drop_keep!(2, 4)),
            (vec![7, 100, 20, 3], vec![7], drop_keep!(3, 0)),
            (vec![100, 20, 120], vec![120], drop_keep!(2, 1)),
            (vec![1, 2, 3, 4, 5], vec![5], drop_keep!(4, 1)),
        ];
        for (input, output, drop_keep) in tests.iter() {
            let opcodes = translate_drop_keep(*drop_keep).unwrap();
            let mut stack = input.clone();
            for opcode in opcodes.iter() {
                match opcode {
                    Instruction::LocalSet(index) => {
                        let last = stack.last().unwrap();
                        let len = stack.len();
                        *stack.get_mut(len - 1 - index.to_usize()).unwrap() = *last;
                        stack.pop();
                    }
                    Instruction::LocalGet(index) => {
                        let len = stack.len();
                        let item = *stack.get(len - index.to_usize()).unwrap();
                        stack.push(item);
                    }
                    Instruction::Drop => {
                        stack.pop();
                    }
                    _ => unreachable!("unknown opcode: {:?}", opcode),
                }
            }
            assert_eq!(stack, *output);
        }
    }
}
