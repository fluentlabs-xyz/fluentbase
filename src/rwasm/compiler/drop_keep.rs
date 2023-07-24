use crate::compiler::Translator;
use wazm_core::{DropKeep, Index, InstructionSet, OpCode, WazmResult};

pub(crate) fn translate_drop_keep(drop_keep: DropKeep) -> WazmResult<Vec<OpCode>> {
    let mut result = Vec::new();
    let (drop, keep) = (drop_keep.drop(), drop_keep.keep());
    if drop == 0 {
        return Ok(result);
    }
    if drop >= keep {
        (0..keep).for_each(|_| result.push(OpCode::LocalSet(Index::from(drop as u32))));
        (0..(drop - keep)).for_each(|_| result.push(OpCode::Drop));
    } else {
        (0..keep).for_each(|i| {
            result.push(OpCode::LocalGet(Index::from(keep as u32 - i as u32 - 1)));
            result.push(OpCode::LocalSet(Index::from(keep as u32 + drop as u32 - i as u32)));
        });
        (0..(keep - drop)).for_each(|_| result.push(OpCode::Drop));
    }
    Ok(result)
}

impl Translator for wazm_wasmi::DropKeep {
    fn translate(&self, result: &mut InstructionSet) -> WazmResult<()> {
        let drop_keep_opcodes = translate_drop_keep(*self)?;
        result.0.extend(&drop_keep_opcodes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::drop_keep::translate_drop_keep;
    use wazm_core::{DropKeep, OpCode};

    #[test]
    fn test_drop_keep_translation() {
        macro_rules! drop_keep {
            ($drop:literal, $keep:literal) => {
                DropKeep::new($drop, $keep)
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
        ];
        for (input, output, drop_keep) in tests.iter() {
            let opcodes = translate_drop_keep(*drop_keep).unwrap();
            let mut stack = input.clone();
            for opcode in opcodes.iter() {
                match opcode {
                    OpCode::LocalSet(index) => {
                        let last = stack.last().unwrap();
                        let len = stack.len();
                        *stack.get_mut(len - 1 - index.0 as usize).unwrap() = *last;
                        stack.pop();
                    }
                    OpCode::LocalGet(index) => {
                        let len = stack.len();
                        let item = *stack.get(len - 1 - index.0 as usize).unwrap();
                        stack.push(item);
                    }
                    OpCode::Drop => {
                        stack.pop();
                    }
                    _ => unreachable!("unknown opcode: {:?}", opcode),
                }
            }
            assert_eq!(stack, *output);
        }
    }
}
