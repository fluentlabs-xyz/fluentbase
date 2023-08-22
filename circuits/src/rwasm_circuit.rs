use crate::constraint_builder::{
    AdviceColumn,
    ConstraintBuilder,
    FixedColumn,
    Query,
    SelectorColumn,
};
use fluentbase_rwasm::{
    engine::bytecode::Instruction,
    rwasm::{BinaryFormatError, ReducedModuleReader},
};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, Region, SimpleFloorPlanner},
    plonk::{Circuit, ConstraintSystem, Error},
};
use std::marker::PhantomData;
use strum::IntoEnumIterator;

const N_OPCODE_LOOKUP_TABLE: usize = 3;
const N_RWASM_LOOKUP_TABLE: usize = 3;

pub trait RwasmLookup<F: FieldExt> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE];
}

#[derive(Clone)]
pub struct RwasmCircuitConfig<F: FieldExt> {
    // selectors
    q_enable: SelectorColumn,
    q_first: SelectorColumn,
    q_last: SelectorColumn,
    // columns
    offset: AdviceColumn,
    code: AdviceColumn,
    aux_size: AdviceColumn,
    aux: AdviceColumn,
    illegal_opcode: SelectorColumn,
    need_more: SelectorColumn,
    // lookup tables
    opcode_table: [FixedColumn; N_OPCODE_LOOKUP_TABLE],
    _pd: PhantomData<F>,
}

impl<F: FieldExt> RwasmLookup<F> for RwasmCircuitConfig<F> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE] {
        unreachable!("not implemented yet");
    }
}

impl<F: FieldExt> RwasmCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let q_first = SelectorColumn(cs.fixed_column());
        let q_last = SelectorColumn(cs.fixed_column());

        let mut cb = ConstraintBuilder::new(q_enable);

        let offset = cb.advice_column(cs);
        let code = cb.advice_column(cs);
        let aux_size = cb.advice_column(cs);
        let aux = cb.advice_column(cs);
        let illegal_opcode = SelectorColumn(cs.fixed_column());
        let need_more = SelectorColumn(cs.fixed_column());
        let opcode_table = cb.fixed_columns(cs);

        // first row always starts with 0 offset
        cb.condition(q_first.current(), |cb| {
            cb.assert_zero("if (q_first) offset is 0", offset.current());
        });

        // if row is not last
        cb.condition(!q_last.current(), |cb| {
            // next offset is current offset plus aux size
            cb.assert_equal(
                "offset+aux_size+1=next_offset",
                offset.current() + aux_size.current() + 1,
                offset.next(),
            );
        });

        // make sure code is in the range and opcode status is correct
        cb.condition(need_more.current(), |cb| {
            // for `q_need_more` selector we don't know exact `aux_size`, but still lets check
            cb.add_lookup(
                "lookup_opcode(code,aux_size,illegal_opcode)",
                [code.current(), opcode_table[1].current(), 0.into()],
                opcode_table.map(|v| v.current()),
            );
        });
        cb.condition(!need_more.current(), |cb| {
            cb.add_lookup(
                "lookup_opcode(code,aux_size,illegal_opcode)",
                [
                    code.current(),
                    aux_size.current(),
                    illegal_opcode.current().0,
                ],
                opcode_table.map(|v| v.current()),
            );
        });

        // if we have error then it's always last row
        cb.condition(illegal_opcode.current().or(need_more.current()), |cb| {
            cb.assert("if (q_need_more) q_last is 1", q_last.current());
        });

        cb.build(cs);

        Self {
            q_enable,
            q_first,
            q_last,
            offset,
            code,
            aux_size,
            aux,
            illegal_opcode,
            need_more,
            opcode_table,
            _pd: Default::default(),
        }
    }

    pub fn load(&mut self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "opcode table",
            |mut region| {
                (0..=0xff).for_each(|offset| {
                    self.opcode_table[0].assign(&mut region, offset, F::from(offset as u64));
                    self.opcode_table[1].assign(&mut region, offset, F::from(0u64));
                    self.opcode_table[2].assign(&mut region, offset, F::from(1u64));
                });
                for (offset, instr) in Instruction::iter().enumerate() {
                    if !instr.is_supported() {
                        continue;
                    }
                    let (code, aux_len) = instr.info();
                    self.opcode_table[0].assign(&mut region, offset, F::from(code as u64));
                    self.opcode_table[1].assign(&mut region, offset, F::from(aux_len as u64));
                    self.opcode_table[2].assign(&mut region, offset, F::from(0u64));
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    pub fn assign_internal(
        &mut self,
        region: &mut Region<'_, F>,
        mut offset: usize,
        bytecode: &[u8],
    ) -> usize {
        let mut module_reader = ReducedModuleReader::new(bytecode);
        self.q_first.enable(region, offset);
        let mut last_row_offset = offset;
        loop {
            let trace = match module_reader.trace_opcode() {
                Some(state) => state,
                None => break,
            };
            self.q_enable.enable(region, offset);
            println!("{:?}", trace);
            self.offset
                .assign(region, offset, F::from(trace.offset as u64));
            self.code.assign(region, offset, F::from(trace.code as u64));
            self.aux_size
                .assign(region, offset, F::from(trace.aux_size as u64));
            self.aux
                .assign(region, offset, F::from(trace.aux.to_bits()));
            if let Err(e) = trace.instr {
                match e {
                    BinaryFormatError::NeedMore(_) => {
                        self.need_more.enable(region, offset);
                    }
                    BinaryFormatError::IllegalOpcode(_) => {
                        self.illegal_opcode.enable(region, offset);
                    }
                }
            }
            last_row_offset = offset;
            offset += 1;
        }
        self.q_last.enable(region, last_row_offset);
        last_row_offset
    }

    pub fn assign(
        &mut self,
        layouter: &mut impl Layouter<F>,
        bytecodes: &Vec<&[u8]>,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "bytecode",
            |mut region| {
                let mut offset = 0;
                for bytecode in bytecodes.iter().copied() {
                    offset = self.assign_internal(&mut region, offset, bytecode);
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

#[derive(Default)]
pub struct RwasmCircuit<'a, F: FieldExt> {
    bytecodes: Vec<&'a [u8]>,
    _pd: PhantomData<F>,
}

impl<'a, F: FieldExt> Circuit<F> for RwasmCircuit<'a, F> {
    type Config = RwasmCircuitConfig<F>;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
        Self::Config::configure(meta)
    }

    fn synthesize(
        &self,
        mut config: Self::Config,
        mut layouter: impl Layouter<F>,
    ) -> Result<(), Error> {
        config.load(&mut layouter)?;
        config.assign(&mut layouter, &self.bytecodes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::rwasm_circuit::RwasmCircuit;
    use fluentbase_rwasm::instruction_set;
    use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

    fn test_ok<I: Into<Vec<u8>>>(bytecode: I) {
        let bytecode: Vec<u8> = bytecode.into();
        let circuit = RwasmCircuit {
            bytecodes: vec![bytecode.as_slice()],
            _pd: Default::default(),
        };
        let k = 10;
        let prover = MockProver::<Fr>::run(k, &circuit, vec![]).unwrap();
        prover.assert_satisfied();
    }

    #[test]
    fn test_add_three_numbers() {
        test_ok(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
            .op_i32_const(3)
            .op_i32_add()
            .op_drop()
        ));
    }

    #[test]
    fn test_illegal_opcode() {
        let bytecode = vec![0xf3];
        test_ok(bytecode);
    }

    #[test]
    fn test_need_more() {
        // 63 is `i32.const` code, it should has 4 bytes after
        let bytecode = vec![63];
        test_ok(bytecode);
        // 63 is `i32.const` code, it should has 4 bytes after
        let bytecode = vec![63, 0x00, 0x00, 0x00];
        test_ok(bytecode);
        // 63 is `i32.const` code, it should has 4 bytes after
        let bytecode = vec![63, 0x00, 0x00, 0x00, 0x00];
        test_ok(bytecode);
    }
}
