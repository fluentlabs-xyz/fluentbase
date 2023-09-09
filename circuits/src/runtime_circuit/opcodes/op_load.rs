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
use fluentbase_rwasm::engine::bytecode::{AddressOffset, Instruction};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpLoadGadget<F> {
    is_i32_load: SelectorColumn,
    is_i64_load: SelectorColumn,
    is_f32_load: SelectorColumn,
    is_f64_load: SelectorColumn,
    is_i32_load8s: SelectorColumn,
    is_i32_load8u: SelectorColumn,
    is_i32_load16s: SelectorColumn,
    is_i32_load16u: SelectorColumn,
    is_i64_load8s: SelectorColumn,
    is_i64_load8u: SelectorColumn,
    is_i64_load16s: SelectorColumn,
    is_i64_load16u: SelectorColumn,
    is_i64_load32s: SelectorColumn,
    is_i64_load32u: SelectorColumn,

    value: AdviceColumn,
    value_as_bytes: [AdviceColumn; Instruction::BYTE_LEN_MAX],
    address: AdviceColumn,
    address_base_offset: AdviceColumn,

    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLoadGadget<F> {
    const NAME: &'static str = "WASM_LOAD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOAD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_i32_load = cb.query_selector();
        let is_i64_load = cb.query_selector();
        let is_f32_load = cb.query_selector();
        let is_f64_load = cb.query_selector();
        let is_i32_load8s = cb.query_selector();
        let is_i32_load8u = cb.query_selector();
        let is_i32_load16s = cb.query_selector();
        let is_i32_load16u = cb.query_selector();
        let is_i64_load8s = cb.query_selector();
        let is_i64_load8u = cb.query_selector();
        let is_i64_load16s = cb.query_selector();
        let is_i64_load16u = cb.query_selector();
        let is_i64_load32s = cb.query_selector();
        let is_i64_load32u = cb.query_selector();

        let value = cb.query_cell();
        let value_as_bytes = cb.query_cells();
        let address = cb.query_cell();
        let address_base_offset = cb.query_cell();

        cb.require_exactly_one_selector(
            [
                is_i32_load,
                is_i64_load,
                is_f32_load,
                is_f64_load,
                is_i32_load8s,
                is_i32_load8u,
                is_i32_load16s,
                is_i32_load16u,
                is_i64_load8s,
                is_i64_load8u,
                is_i64_load16s,
                is_i64_load16u,
                is_i64_load32s,
                is_i64_load32u,
            ]
            .map(|v| v.current().0),
        );

        let value_as_bytes_sum = value_as_bytes
            .iter()
            .rev()
            .fold(Query::zero(), |a, v| a * Query::from(0x100) + v.current());
        cb.require_zero(
            "value=recover_from_bytes(value_as_bytes)",
            value.current() - value_as_bytes_sum,
        );

        cb.stack_pop(address.current());
        let mut constrain_instr = |selector: Query<F>, instr: &Instruction| {
            cb.if_rwasm_opcode(selector, *instr, |cb| {
                (0..Instruction::load_instr_meta(instr).0).for_each(|i| {
                    cb.mem_read(
                        address_base_offset.current() + address.current() + i.expr(),
                        value_as_bytes[i].current(),
                    );
                });
            })
        };

        [
            (is_i32_load, Instruction::I32Load(Default::default())),
            (is_i64_load, Instruction::I64Load(Default::default())),
            (is_f32_load, Instruction::F32Load(Default::default())),
            (is_f64_load, Instruction::F64Load(Default::default())),
            (is_i32_load8s, Instruction::I32Load8S(Default::default())),
            (is_i32_load8u, Instruction::I32Load8U(Default::default())),
            (is_i32_load16s, Instruction::I32Load16S(Default::default())),
            (is_i32_load16u, Instruction::I32Load16U(Default::default())),
            (is_i64_load8s, Instruction::I64Load8S(Default::default())),
            (is_i64_load8u, Instruction::I64Load8U(Default::default())),
            (is_i64_load16s, Instruction::I64Load16S(Default::default())),
            (is_i64_load16u, Instruction::I64Load16U(Default::default())),
            (is_i64_load32s, Instruction::I64Load32S(Default::default())),
            (is_i64_load32u, Instruction::I64Load32U(Default::default())),
        ]
        .map(|v| (v.0.current().0, v.1))
        .iter()
        .for_each(|v| constrain_instr(v.0.clone(), &v.1));
        cb.stack_push(value.current());

        Self {
            is_i32_load,
            is_i64_load,
            is_f32_load,
            is_f64_load,
            is_i32_load8s,
            is_i32_load8u,
            is_i32_load16s,
            is_i32_load16u,
            is_i64_load8s,
            is_i64_load8u,
            is_i64_load16s,
            is_i64_load16u,
            is_i64_load32s,
            is_i64_load32u,
            value,
            value_as_bytes,
            address,
            address_base_offset,
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let address = trace.curr_nth_stack_value(0)?.to_bits();

        let instr = trace.instr();

        let mut assign = |selector: &SelectorColumn,
                          address_offset: &AddressOffset|
         -> Result<(), GadgetError> {
            selector.enable(region, offset);

            let byte_len = Instruction::load_instr_meta(instr).0 as usize;
            let mut value_le_bytes = vec![0; byte_len];
            let mem_address_base = address_offset.into_inner() as u64 + address;
            trace.curr_read_memory(
                mem_address_base,
                value_le_bytes.as_mut_ptr(),
                byte_len as u32,
            )?;

            let mut value: u64 = 0;
            for byte in value_le_bytes.iter().copied().rev() {
                // TODO what about sign?
                value = value * 0x100 + byte as u64;
            }

            self.value.assign(region, offset, value);
            for (i, byte_val) in value_le_bytes.iter().enumerate() {
                self.value_as_bytes[i].assign(region, offset, *byte_val as u64);
            }
            self.address.assign(region, offset, address);
            self.address_base_offset
                .assign(region, offset, address_offset.into_inner() as u64);
            Ok(())
        };

        match instr {
            Instruction::I32Load(address_offset) => {
                assign(&self.is_i32_load, address_offset)?;
            }
            Instruction::I64Load(address_offset) => {
                assign(&self.is_i64_load, address_offset)?;
            }
            Instruction::F32Load(address_offset) => {
                assign(&self.is_f32_load, address_offset)?;
            }
            Instruction::F64Load(address_offset) => {
                assign(&self.is_f64_load, address_offset)?;
            }
            Instruction::I32Load8S(address_offset) => {
                assign(&self.is_i32_load8s, address_offset)?;
            }
            Instruction::I32Load8U(address_offset) => {
                assign(&self.is_i32_load8u, address_offset)?;
            }
            Instruction::I32Load16S(address_offset) => {
                assign(&self.is_i32_load16s, address_offset)?;
            }
            Instruction::I32Load16U(address_offset) => {
                assign(&self.is_i32_load16u, address_offset)?;
            }
            Instruction::I64Load8S(address_offset) => {
                assign(&self.is_i64_load8s, address_offset)?;
            }
            Instruction::I64Load8U(address_offset) => {
                assign(&self.is_i64_load8u, address_offset)?;
            }
            Instruction::I64Load16S(address_offset) => {
                assign(&self.is_i64_load16s, address_offset)?;
            }
            Instruction::I64Load16U(address_offset) => {
                assign(&self.is_i64_load16u, address_offset)?;
            }
            Instruction::I64Load32S(address_offset) => {
                assign(&self.is_i64_load32s, address_offset)?;
            }
            Instruction::I64Load32U(address_offset) => {
                assign(&self.is_i64_load32u, address_offset)?;
            }
            _ => unreachable!("illegal opcode assign {:?}", instr),
        };

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;
    use rand::{thread_rng, Rng};

    fn gen_address_params() -> [u32; 2] {
        [
            thread_rng().gen_range(0..100),
            thread_rng().gen_range(0..100),
        ]
    }

    #[test]
    fn test_i32_load() {
        let [address_offset, address] = [1, 0];
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[800]
            I32Store[address_offset]

            I32Const[address]
            I32Load[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i32_load8u() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[15]
            I32Store8[address_offset]

            I32Const[address]
            I32Load8U[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i32_load8s_1() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[13]
            I32Store8[address_offset]

            I32Const[address]
            I32Load8S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i32_load8s_2() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[14]
            I32Store8[address_offset]

            I32Const[address]
            I32Load8S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i32_load16u() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[801]
            I32Store[address_offset]

            I32Const[address]
            I32Load16U[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i32_load16s() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[802]
            I32Store[address_offset]

            I32Const[address]
            I32Load16S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[803]
            I64Store[address_offset]

            I64Const[address]
            I64Load[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load8u() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[21]
            I64Store8[address_offset]

            I64Const[address]
            I64Load8U[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load8s() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[22]
            I64Store8[address_offset]

            I64Const[address]
            I64Load8S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load16u() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[22]
            I64Store[address_offset]

            I64Const[address]
            I64Load16U[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load16s() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[807]
            I64Store[address_offset]

            I64Const[address]
            I64Load16S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load32u() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[808]
            I64Store[address_offset]

            I64Const[address]
            I64Load32U[address_offset]
            Drop
        });
    }

    #[test]
    fn test_i64_load32s() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[809]
            I64Store[address_offset]

            I64Const[address]
            I64Load32S[address_offset]
            Drop
        });
    }

    #[test]
    fn test_f32_load() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I32Const[address]
            I32Const[20]
            F32Store[address_offset]

            I32Const[address]
            F32Load[address_offset]
            Drop
        });
    }

    #[test]
    fn test_f64_load() {
        let [address_offset, address] = gen_address_params();
        test_ok(instruction_set! {
            I64Const[address]
            I64Const[810]
            I64Store[address_offset]

            I64Const[address]
            F64Load[address_offset]
            Drop
        });
    }
}
