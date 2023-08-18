use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, FixedColumn, SelectorColumn},
    gadgets::byte_bit::RangeCheck256Lookup,
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

#[derive(Clone)]
pub struct RwasmCircuitConfig<F: FieldExt> {
    // selectors
    q_enable: SelectorColumn,
    q_first: SelectorColumn,
    q_last: SelectorColumn,
    q_illegal_opcode: SelectorColumn,
    q_need_more: SelectorColumn,
    // columns
    offset: AdviceColumn,
    code: AdviceColumn,
    aux_size: AdviceColumn,
    aux: AdviceColumn,
    // lookup tables
    opcode_table: [FixedColumn; 3],
    _pd: PhantomData<F>,
}

impl<F: FieldExt> RwasmCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let q_first = SelectorColumn(cs.fixed_column());
        let q_last = SelectorColumn(cs.fixed_column());
        let q_illegal_opcode = SelectorColumn(cs.fixed_column());
        let q_need_more = SelectorColumn(cs.fixed_column());

        let mut cb = ConstraintBuilder::new(q_enable);

        let offset = cb.advice_column(cs);
        let code = cb.advice_column(cs);
        let aux_size = cb.advice_column(cs);
        let aux = cb.advice_column(cs);
        let opcode_table = cb.fixed_columns(cs);

        cb.condition(q_first.current(), |cb| {
            cb.assert_zero("if (q_first) offset is 0", offset.current());
        });

        // make sure code is in the range and opcode status is correct
        cb.add_lookup(
            "lookup_opcode(code,hint)",
            [
                code.current(),
                aux_size.current(),
                q_illegal_opcode.current().0,
            ],
            opcode_table.map(|v| v.current()),
        );

        // if we have error then it's always last row
        cb.condition(q_illegal_opcode.current().or(q_need_more.current()), |cb| {
            cb.assert("if (q_need_more) q_last(next) is 0", q_last.next());
        });

        Self {
            q_enable,
            q_first,
            q_last,
            q_illegal_opcode,
            q_need_more,
            offset,
            code,
            aux_size,
            aux,
            opcode_table,
            _pd: Default::default(),
        }
    }

    pub fn load(&mut self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "opcode table",
            |mut region| {
                (0..=0xff).for_each(|offset| {
                    self.opcode_table[0].assign(&mut region, offset, F::from(0u64));
                    self.opcode_table[1].assign(&mut region, offset, F::from(0u64));
                    self.opcode_table[2].assign(&mut region, offset, F::from(1u64));
                });
                for (offset, instr) in Instruction::iter().enumerate() {
                    if !instr.is_supported() {
                        continue;
                    }
                    let (code, hint) = instr.info();
                    self.opcode_table[0].assign(&mut region, offset, F::from(code as u64));
                    self.opcode_table[1].assign(&mut region, offset, F::from(hint as u64));
                    self.opcode_table[2].assign(&mut region, offset, F::from(1u64));
                }
                Ok(())
            },
        )?;
        Ok(())
    }

    pub fn assign_internal(
        &mut self,
        region: &mut Region<'_, F>,
        offset: &mut usize,
        bytecode: &[u8],
    ) {
        let mut module_reader = ReducedModuleReader::new(bytecode);
        self.q_first.enable(region, *offset);
        loop {
            self.q_enable.enable(region, *offset);
            let trace = match module_reader.trace_opcode() {
                Some(state) => state,
                None => break,
            };
            self.offset
                .assign(region, *offset, F::from(trace.offset as u64));
            self.code
                .assign(region, *offset, F::from(trace.code as u64));
            self.aux_size
                .assign(region, *offset, F::from(trace.aux_size as u64));
            self.aux
                .assign(region, *offset, F::from(trace.aux.to_bits()));
            if let Err(e) = trace.instr {
                match e {
                    BinaryFormatError::NeedMore(_) => {
                        self.q_need_more.enable(region, *offset);
                    }
                    BinaryFormatError::IllegalOpcode(_) => {
                        self.q_illegal_opcode.enable(region, *offset);
                    }
                }
            }
            *offset += 1;
        }
        self.q_last.enable(region, *offset);
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
                    self.assign_internal(&mut region, &mut offset, bytecode);
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
    use fluentbase_rwasm::{
        instruction_set,
        rwasm::{BinaryFormat, BinaryFormatWriter, InstructionSet},
    };
    use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

    fn bytecode_to_bytes(bytecode: InstructionSet) -> Vec<u8> {
        let mut buffer = vec![0; 1024];
        let mut binary_writer = BinaryFormatWriter::new(buffer.as_mut_slice());
        let n = bytecode.write_binary(&mut binary_writer).unwrap();
        buffer.resize(n, 0);
        buffer
    }

    #[test]
    fn simple_test() {
        let bytecode = bytecode_to_bytes(instruction_set!(
            .op_i32_const(100)
            .op_i32_const(20)
            .op_i32_add()
        ));
        let circuit = RwasmCircuit {
            bytecodes: vec![bytecode.as_slice()],
            _pd: Default::default(),
        };
        let k = 10;
        let is_ok = true;
        let prover = MockProver::<Fr>::run(k, &circuit, vec![]).unwrap();
        if is_ok {
            prover.assert_satisfied();
        } else {
            assert!(prover.verify().is_err());
        }
    }
}
