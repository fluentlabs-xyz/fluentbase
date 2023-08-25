use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, FixedColumn, Query, SelectorColumn},
    gadgets::poseidon::PoseidonTable,
    poseidon_circuit::HASH_BYTES_IN_FIELD,
    unrolled_bytecode::UnrolledBytecode,
    util::Field,
};
use ethers_core::types::U256;
use fluentbase_rwasm::{
    engine::bytecode::Instruction,
    rwasm::{BinaryFormatError, ReducedModuleTrace},
};
use halo2_proofs::{
    circuit::{Layouter, Region},
    plonk::{ConstraintSystem, Error},
};
use std::marker::PhantomData;
use strum::IntoEnumIterator;

const N_OPCODE_LOOKUP_TABLE: usize = 3;
const N_RWASM_LOOKUP_TABLE: usize = 3;

pub trait RwasmLookup<F: Field> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE];
}

pub type OpcodeTable = [FixedColumn; N_OPCODE_LOOKUP_TABLE];

#[derive(Clone)]
pub struct RwasmCircuitConfig<F: Field> {
    // selectors
    q_enable: SelectorColumn,
    q_first: SelectorColumn,
    q_last: SelectorColumn,
    // columns
    offset: AdviceColumn,
    code: AdviceColumn,
    aux_size: AdviceColumn,
    aux: AdviceColumn,
    reached_unreachable: SelectorColumn,
    need_more: SelectorColumn,
    illegal_opcode: SelectorColumn,
    code_hash: AdviceColumn,
    lookup_hash: SelectorColumn,
    field_input: AdviceColumn,
    // lookup tables
    poseidon_table: PoseidonTable,
    opcode_table: OpcodeTable,
    _pd: PhantomData<F>,
}

impl<F: Field> RwasmLookup<F> for RwasmCircuitConfig<F> {
    fn lookup_rwasm_table(&self) -> [Query<F>; N_RWASM_LOOKUP_TABLE] {
        unreachable!("not implemented yet");
    }
}

impl<F: Field> RwasmCircuitConfig<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>, poseidon_table: PoseidonTable) -> Self {
        let q_enable = SelectorColumn(cs.fixed_column());
        let q_first = SelectorColumn(cs.fixed_column());
        let q_last = SelectorColumn(cs.fixed_column());

        let mut cb = ConstraintBuilder::new(q_enable);

        let offset = cb.advice_column(cs);
        let code = cb.advice_column(cs);
        let aux_size = cb.advice_column(cs);
        let aux = cb.advice_column(cs);
        let reached_unreachable = SelectorColumn(cs.fixed_column());
        let need_more = SelectorColumn(cs.fixed_column());
        let illegal_opcode = SelectorColumn(cs.fixed_column());
        let code_hash = cb.advice_column(cs);
        let lookup_hash = SelectorColumn(cs.fixed_column());
        let field_input = cb.advice_column(cs);

        let opcode_table = cb.fixed_columns(cs);

        let if_error = || {
            reached_unreachable
                .current()
                .or(need_more.current())
                .or(illegal_opcode.current())
        };

        // if row is not last
        cb.condition(!q_last.current(), |cb| {
            // next offset is current offset plus aux size
            cb.assert_equal(
                "next_offset+aux_size+1=offset",
                offset.current(),
                offset.next() + aux_size.current() + 1,
            );
            cb.assert_equal(
                "cur_code_hash=next_code_hash",
                code_hash.current(),
                code_hash.next(),
            );
        });
        cb.condition(q_last.current().and(!if_error()), |cb| {
            cb.assert_equal(
                "offset is aligned",
                offset.current(),
                Query::from(HASH_BYTES_IN_FIELD as u64),
            );
        });

        // make sure code is in the range and opcode status is correct
        cb.condition(illegal_opcode.current(), |cb| {
            cb.add_lookup(
                "lookup_opcode(code,aux_size,error)",
                [code.current(), 0.into(), 1.into()],
                opcode_table.map(|v| v.current()),
            );
        });
        cb.condition(need_more.current(), |cb| {
            // for `q_need_more` selector we don't know exact `aux_size`, but still lets check
            cb.add_lookup(
                "lookup_opcode(code,aux_size,error)",
                [code.current(), opcode_table[1].current(), 0.into()],
                opcode_table.map(|v| v.current()),
            );
        });
        cb.condition(!need_more.current(), |cb| {
            cb.add_lookup(
                "lookup_opcode(code,aux_size,error)",
                [
                    code.current(),
                    aux_size.current(),
                    illegal_opcode.current().0,
                ],
                opcode_table.map(|v| v.current()),
            );
        });

        // if we have error then it's always last row
        cb.condition(
            illegal_opcode
                .current()
                .or(need_more.current())
                .or(reached_unreachable.current()),
            |cb| {
                cb.assert("if (q_need_more) q_last is 1", q_last.current());
            },
        );

        // lookup poseidon state
        cb.condition(lookup_hash.current(), |cb| {
            cb.poseidon_lookup(
                "poseidon_lookup(code,aux,code_hash)",
                code_hash.current(),   // code hash
                field_input.current(), // left
                field_input.next(),    // right
                offset.current(),      // offset
                &poseidon_table,
            );
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
            reached_unreachable,
            need_more,
            illegal_opcode,
            code_hash,
            lookup_hash,
            field_input,
            poseidon_table,
            opcode_table,
            _pd: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
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

    pub fn assign_trace(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        code_hash: F,
        trace: &ReducedModuleTrace,
    ) {
        self.q_enable.enable(region, offset);
        println!("{:?}", trace);
        self.offset.assign(
            region,
            offset,
            F::from((trace.bytecode_length - trace.offset) as u64),
        );
        self.code.assign(region, offset, F::from(trace.code as u64));
        self.aux_size
            .assign(region, offset, F::from(trace.aux_size as u64));
        self.aux
            .assign(region, offset, F::from(trace.aux.to_bits()));
        debug_assert_eq!(HASH_BYTES_IN_FIELD, 9);
        let mut raw_field: [u8; 64] = [0; 64];
        U256::from_big_endian(&trace.raw_bytes).to_little_endian(&mut raw_field[0..32]);
        let field_input = F::from_bytes_wide(&raw_field);
        self.field_input.assign(region, offset, field_input);
        if let Err(e) = trace.instr {
            match e {
                BinaryFormatError::ReachedUnreachable => {
                    self.reached_unreachable.enable(region, offset);
                }
                BinaryFormatError::NeedMore(_) => {
                    self.need_more.enable(region, offset);
                }
                BinaryFormatError::IllegalOpcode(_) => {
                    self.illegal_opcode.enable(region, offset);
                }
            }
        }
        self.code_hash.assign(region, offset, code_hash);
        // determine input side for poseidon hash (left or right)
        if (trace.offset / HASH_BYTES_IN_FIELD) % 2 == 0 {
            self.lookup_hash.enable(region, offset);
        }
    }

    pub fn assign_bytecode(
        &self,
        region: &mut Region<'_, F>,
        mut offset: usize,
        bytecode: &UnrolledBytecode<F>,
    ) -> usize {
        self.q_first.enable(region, offset);
        let mut last_row_offset = offset;
        let code_hash = bytecode.code_hash();
        for trace in bytecode.read_traces() {
            last_row_offset = offset;
            self.assign_trace(region, offset, code_hash.clone(), &trace);
            offset += 1;
        }
        self.q_last.enable(region, last_row_offset);
        last_row_offset
    }

    pub fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        bytecode: &UnrolledBytecode<F>,
    ) -> Result<(), Error> {
        layouter.assign_region(
            || "bytecode",
            |mut region| {
                self.assign_bytecode(&mut region, 0, bytecode);
                Ok(())
            },
        )?;
        Ok(())
    }
}
