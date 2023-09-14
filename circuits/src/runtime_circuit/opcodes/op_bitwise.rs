use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::{marker::PhantomData, ops::Add};

pub const LIMBS_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct OpBitwiseGadget<F> {
    is_i32_and: SelectorColumn,
    is_i64_and: SelectorColumn,
    is_i32_or: SelectorColumn,
    is_i64_or: SelectorColumn,
    is_i32_xor: SelectorColumn,
    is_i64_xor: SelectorColumn,

    p1: AdviceColumn,
    p2: AdviceColumn,
    r: AdviceColumn,

    p1_bytes: [AdviceColumn; LIMBS_COUNT],
    p2_bytes: [AdviceColumn; LIMBS_COUNT],
    r_bytes: [AdviceColumn; LIMBS_COUNT],

    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpBitwiseGadget<F> {
    const NAME: &'static str = "WASM_BITWISE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_BITWISE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32_and = cb.query_selector();
        let is_i64_and = cb.query_selector();
        let is_i32_or = cb.query_selector();
        let is_i64_or = cb.query_selector();
        let is_i32_xor = cb.query_selector();
        let is_i64_xor = cb.query_selector();

        let [p1, p2, r] = cb.query_cells();

        let p1_bytes = cb.query_cells();
        let p2_bytes = cb.query_cells();
        let r_bytes = cb.query_cells();

        cb.require_exactly_one_selector(
            [
                is_i32_and, is_i64_and, is_i32_or, is_i64_or, is_i32_xor, is_i64_xor,
            ]
            .map(|v| v.current().0),
        );

        let mut constrain_val =
            |name: &'static str, column: &AdviceColumn, bytes: &[AdviceColumn; LIMBS_COUNT]| {
                cb.require_equal(
                    name,
                    column.current(),
                    bytes
                        .iter()
                        .rev()
                        .fold(Query::zero(), |a, v| a * Query::from(0x100) + v.current()),
                );
            };
        [
            ("p1=reconstructed(p1_bytes)", p1, p1_bytes),
            ("p2=reconstructed(p2_bytes)", p2, p2_bytes),
            ("r=reconstructed(r_bytes)", r, r_bytes),
        ]
        .iter()
        .for_each(|v| constrain_val(v.0, &v.1, &v.2));

        let mut constrain_instr = |instr: &Instruction| {
            let sel = match instr {
                Instruction::I32And => is_i32_and.current(),
                Instruction::I64And => is_i64_and.current(),
                Instruction::I32Or => is_i32_or.current(),
                Instruction::I64Or => is_i64_or.current(),
                Instruction::I32Xor => is_i32_xor.current(),
                Instruction::I64Xor => is_i64_xor.current(),
                _ => unreachable!("configure: unsupported bitwise opcode {:?}", instr),
            };
            cb.if_rwasm_opcode(sel.0.clone(), *instr, |cb| {
                (0..LIMBS_COUNT).for_each(|i| {
                    match instr {
                        Instruction::I32And | Instruction::I64And => cb.bitwise_and(
                            p1_bytes[i].current() * sel.clone(),
                            p2_bytes[i].current() * sel.clone(),
                            r_bytes[i].current() * sel.clone(),
                        ),
                        Instruction::I32Or | Instruction::I64Or => cb.bitwise_or(
                            p1_bytes[i].current() * sel.clone(),
                            p2_bytes[i].current() * sel.clone(),
                            r_bytes[i].current() * sel.clone(),
                        ),
                        Instruction::I32Xor | Instruction::I64Xor => cb.bitwise_xor(
                            p1_bytes[i].current() * sel.clone(),
                            p2_bytes[i].current() * sel.clone(),
                            r_bytes[i].current() * sel.clone(),
                        ),
                        _ => unreachable!("configure: unsupported bitwise opcode {:?}", instr),
                    };
                });
            })
        };

        [
            Instruction::I32And,
            Instruction::I64And,
            Instruction::I32Or,
            Instruction::I64Or,
            Instruction::I32Xor,
            Instruction::I64Xor,
        ]
        .iter()
        .for_each(|instr| constrain_instr(instr));

        cb.stack_pop(p1.current());
        cb.stack_pop(p2.current());
        cb.stack_push(r.current());

        Self {
            is_i32_and,
            is_i64_and,
            is_i32_or,
            is_i64_or,
            is_i32_xor,
            is_i64_xor,
            p1,
            p2,
            r,
            p1_bytes,
            p2_bytes,
            r_bytes,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let p1 = trace.curr_nth_stack_value(0)?.to_bits();
        let p2 = trace.curr_nth_stack_value(1)?.to_bits();
        let r = trace.next_nth_stack_value(0)?.to_bits();

        let p1_bytes = p1.to_le_bytes();
        let p2_bytes = p2.to_le_bytes();
        let r_bytes = r.to_le_bytes();

        let opcode = &trace.trace.opcode;
        match opcode {
            Instruction::I32And => self.is_i32_and.enable(region, offset),
            Instruction::I64And => self.is_i64_and.enable(region, offset),
            Instruction::I32Or => self.is_i32_or.enable(region, offset),
            Instruction::I64Or => self.is_i64_or.enable(region, offset),
            Instruction::I32Xor => self.is_i32_xor.enable(region, offset),
            Instruction::I64Xor => self.is_i64_xor.enable(region, offset),
            _ => unreachable!("assign: unsupported bitwise opcode {:?}", opcode),
        }

        self.p1.assign(region, offset, p1);
        self.p2.assign(region, offset, p2);
        self.r.assign(region, offset, r);

        [
            (self.p1_bytes, p1_bytes),
            (self.p2_bytes, p2_bytes),
            (self.r_bytes, r_bytes),
        ]
        .iter()
        .for_each(|(column_bytes, runtime_bytes)| {
            column_bytes.iter().enumerate().for_each(|(i, column)| {
                column.assign(region, offset, runtime_bytes[i] as u64);
            });
        });

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use log::debug;
    use rand::{thread_rng, Rng};
    const MAX: i64 = 10000;

    fn gen_params<const N: usize>() -> [i64; N] {
        let params = [0; N].map(|i| thread_rng().gen_range(0..=MAX * 2) - MAX);
        debug!("params {:?}", params);
        params
    }

    #[test]
    fn test_i32_and() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32And

            Drop
        });
    }

    #[test]
    fn test_i64_and() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I64Const[p1]
            I64Const[p2]
            I64And

            Drop
        });
    }

    #[test]
    fn test_i32_or() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32Or

            Drop
        });
    }

    #[test]
    fn test_i64_or() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I64Const[p1]
            I64Const[p2]
            I64Or

            Drop
        });
    }

    #[test]
    fn test_i32_xor() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I32Const[p1]
            I32Const[p2]
            I32Xor

            Drop
        });
    }

    #[test]
    fn test_i64_xor() {
        let [p1, p2] = gen_params();
        test_ok(instruction_set! {
            I64Const[p1]
            I64Const[p2]
            I64Xor

            Drop
        });
    }
}
