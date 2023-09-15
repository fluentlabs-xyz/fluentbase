use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn},
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
use std::marker::PhantomData;

const LIMBS_COUNT: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct OpExtendGadget<F> {
    is_i32extend8s: SelectorColumn,
    is_i64extend8s: SelectorColumn,
    is_i32extend16s: SelectorColumn,
    is_i64extend16s: SelectorColumn,
    is_i64extend32s: SelectorColumn,
    is_i64extend_i32s: SelectorColumn,
    is_i64extend_i32u: SelectorColumn,

    p: AdviceColumn,
    r: AdviceColumn,

    p_bytes: [AdviceColumn; LIMBS_COUNT],
    p_signs: [AdviceColumn; LIMBS_COUNT],
    r_bytes: [AdviceColumn; LIMBS_COUNT],

    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpExtendGadget<F> {
    const NAME: &'static str = "WASM_EXTEND";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_EXTEND;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32extend8s = cb.query_selector();
        let is_i32extend16s = cb.query_selector();
        let is_i64extend8s = cb.query_selector();
        let is_i64extend16s = cb.query_selector();
        let is_i64extend32s = cb.query_selector();
        let is_i64extend_i32s = cb.query_selector();
        let is_i64extend_i32u = cb.query_selector();

        let p = cb.query_cell();
        let r = cb.query_cell();

        let p_bytes = cb.query_cells();
        let p_signs = cb.query_cells();
        let r_bytes = cb.query_cells();

        cb.require_exactly_one_selector(
            [
                is_i32extend8s,
                is_i64extend8s,
                is_i32extend16s,
                is_i64extend16s,
                is_i64extend32s,
                is_i64extend_i32s,
                is_i64extend_i32u,
            ]
            .map(|v| v.current().0),
        );
        (0..LIMBS_COUNT).for_each(|i| {
            cb.range_check7(p_bytes[i].current());
            cb.require_boolean("p_signs are bool", p_signs[i].current());
            cb.range_check8(r_bytes[i].current());
        });

        cb.stack_pop(p.current());
        cb.stack_push(r.current());

        let mut constrain_val =
            |name: &'static str,
             column: &AdviceColumn,
             bytes: &[AdviceColumn; LIMBS_COUNT],
             signs: Option<&[AdviceColumn; LIMBS_COUNT]>| {
                if let Some(signs) = signs {
                    cb.require_equal(
                        name,
                        column.current(),
                        bytes.iter().zip(signs).rev().fold(Query::zero(), |a, v| {
                            a * Query::from(0x100)
                                + v.0.current()
                                + v.1.current() * Query::from(0b10000000)
                        }),
                    );
                } else {
                    cb.require_equal(
                        name,
                        column.current(),
                        bytes
                            .iter()
                            .rev()
                            .fold(Query::zero(), |a, v| a * Query::from(0x100) + v.current()),
                    );
                }
            };
        [
            (
                "p=reconstructed(p_bytes,p_signs)",
                p,
                p_bytes,
                Some(&p_signs),
            ),
            ("r=reconstructed(r_bytes)", r, r_bytes, None),
        ]
        .iter()
        .for_each(|v| constrain_val(v.0, &v.1, &v.2, v.3));

        let mut constrain_instr = |instr: &Instruction| {
            let sel = match instr {
                Instruction::I32Extend8S => is_i32extend8s.current(),
                Instruction::I64Extend8S => is_i64extend8s.current(),
                Instruction::I32Extend16S => is_i32extend16s.current(),
                Instruction::I64Extend16S => is_i64extend16s.current(),
                Instruction::I64Extend32S => is_i64extend32s.current(),
                Instruction::I64ExtendI32S => is_i64extend_i32s.current(),
                Instruction::I64ExtendI32U => is_i64extend_i32u.current(),
                _ => unreachable!("configure: unsupported extend opcode {:?}", instr),
            };
            cb.if_rwasm_opcode(sel.0.clone(), *instr, |cb| {
                let (ibs, obs) = instr_meta(instr);
                (0..LIMBS_COUNT).for_each(|i| {
                    if i < ibs {
                        cb.require_equal(
                            "p_bytes[0..ibs)+p_signs[i]*0b10000000=r_bytes[0..ibs)",
                            p_bytes[i].current() + p_signs[i].current() * Query::from(0b10000000),
                            r_bytes[i].current(),
                        );
                    } else if i < obs {
                        cb.require_equal(
                            "p_signs[ibs-1)*0b11111111=r_bytes[ibs..rbs)",
                            p_signs[ibs - 1].current()
                                * Query::from(0b11111111)
                                * (Query::one() - is_i64extend_i32u.current()),
                            r_bytes[i].current(),
                        );
                    } else {
                        cb.require_zero("r_bytes[obs..LIMBS_COUNT)=0", r_bytes[i].current());
                    }
                });
            })
        };
        [
            Instruction::I32Extend8S,
            Instruction::I64Extend8S,
            Instruction::I32Extend16S,
            Instruction::I64Extend16S,
            Instruction::I64Extend32S,
            Instruction::I64ExtendI32S,
            Instruction::I64ExtendI32U,
        ]
        .iter()
        .for_each(|instr| constrain_instr(instr));

        Self {
            is_i32extend8s,
            is_i32extend16s,
            is_i64extend8s,
            is_i64extend16s,
            is_i64extend32s,
            is_i64extend_i32s,
            is_i64extend_i32u,
            p,
            r,
            p_bytes,
            p_signs,
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
        let p = trace.curr_nth_stack_value(0)?.to_bits();
        let r = trace.next_nth_stack_value(0)?.to_bits();

        let p_bytes = p.to_le_bytes();
        let r_bytes = r.to_le_bytes();

        let opcode = &trace.trace.opcode;
        match opcode {
            Instruction::I32Extend8S => self.is_i32extend8s.enable(region, offset),
            Instruction::I64Extend8S => self.is_i64extend8s.enable(region, offset),
            Instruction::I32Extend16S => self.is_i32extend16s.enable(region, offset),
            Instruction::I64Extend16S => self.is_i64extend16s.enable(region, offset),
            Instruction::I64Extend32S => self.is_i64extend32s.enable(region, offset),
            Instruction::I64ExtendI32S => self.is_i64extend_i32s.enable(region, offset),
            Instruction::I64ExtendI32U => self.is_i64extend_i32u.enable(region, offset),
            _ => unreachable!("assign: unsupported extend opcode {:?}", opcode),
        }

        self.p.assign(region, offset, p);
        self.r.assign(region, offset, r);

        (0..LIMBS_COUNT).for_each(|i| {
            let p_byte = p_bytes[i];
            self.p_bytes[i].assign(region, offset, (p_byte & 0b1111111) as u64);
            self.p_signs[i].assign(region, offset, (p_byte & 0b10000000 > 0) as u64);
            self.r_bytes[i].assign(region, offset, r_bytes[i] as u64);
        });

        Ok(())
    }
}

type InputByteSize = usize;
type OutputBytesSize = usize;
fn instr_meta(opcode: &Instruction) -> (InputByteSize, OutputBytesSize) {
    match opcode {
        Instruction::I32Extend8S => (1, 4),
        Instruction::I64Extend8S => (1, 8),
        Instruction::I32Extend16S => (2, 4),
        Instruction::I64Extend16S => (2, 8),
        Instruction::I64Extend32S | Instruction::I64ExtendI32S | Instruction::I64ExtendI32U => {
            (4, 8)
        }
        _ => unreachable!("sign_byte_index: unsupported extend opcode {:?}", opcode),
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use log::debug;
    use rand::{thread_rng, Rng};

    fn gen_params<const N: usize, const MAX_POSITIVE_VAL: i64>() -> [i64; N] {
        let params = [0; N]
            .map(|_i| thread_rng().gen_range(0..=MAX_POSITIVE_VAL * 2 + 1) - MAX_POSITIVE_VAL - 1);
        debug!("params {:?}", params);
        params
    }

    // Instruction::I32Extend8S
    #[test]
    fn test_i32extend8s() {
        let [p] = gen_params::<1, 0b1111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I32Extend8S

            Drop
        });
    }

    // Instruction::I64Extend8S
    #[test]
    fn test_i64extend8s() {
        let [p] = gen_params::<1, 0b1111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I64Extend8S

            Drop
        });
    }

    // Instruction::I32Extend16S
    #[test]
    fn test_i32extend16s() {
        let [p] = gen_params::<1, 0b111111111111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I32Extend16S

            Drop
        });
    }

    // Instruction::I64Extend16S
    #[test]
    fn test_i64extend16s() {
        let [p] = gen_params::<1, 0b111111111111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I64Extend16S

            Drop
        });
    }

    // Instruction::I64Extend32S
    #[test]
    fn test_i64extend32s() {
        let [p] = gen_params::<1, 0b1111111111111111111111111111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I64Extend32S

            Drop
        });
    }

    // Instruction::I64ExtendI32S
    #[test]
    fn test_i64extend_i32s() {
        let [p] = gen_params::<1, 0b1111111111111111111111111111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I64ExtendI32S

            Drop
        });
    }

    // Instruction::I64ExtendI32U
    #[test]
    fn test_i64extend_i32u() {
        let [p] = gen_params::<1, 0b1111111111111111111111111111111>();
        test_ok(instruction_set! {
            I64Const[p]
            I64ExtendI32U

            Drop
        });
    }
}
