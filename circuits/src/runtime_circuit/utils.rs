use crate::{
    constraint_builder::{AdviceColumn, Query, SelectorColumn},
    exec_step::GadgetError,
    runtime_circuit::constraint_builder::OpConstraintBuilder,
    util::Field,
};
use halo2_proofs::circuit::Region;
use serde::Serialize;
use std::marker::PhantomData;

#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShiftOp {
    Shl,
    UnsignedShr,
    SignedShr,
    Rotl,
    Rotr,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct U64CellWithFlagBitDyn<F: Field> {
    pub(crate) u64_as_u16_le: [AdviceColumn; 4],
    pub(crate) u64: AdviceColumn,
    pub(crate) sign_bit: AdviceColumn,
    pub(crate) u16_sign_bit_rem: AdviceColumn,
    pub(crate) u16_sign_bit_rem_diff: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> U64CellWithFlagBitDyn<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
        is_i32_otherwise_i64: bool,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u64_as_u16_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64.assign(region, offset, value);

        let sign_bit_idx = if is_i32_otherwise_i64 { 1 } else { 3 };
        let u16_sign_byte = (value >> (sign_bit_idx * 16)) & 0xffff;
        let sign_bit = u16_sign_byte >> 15;
        let u16_sign_bit_rem = u16_sign_byte & 0x7fff;
        let u16_sign_bit_rem_diff = 0x7fff - u16_sign_bit_rem;
        self.sign_bit.assign(region, offset, sign_bit as u32 as u64);
        self.u16_sign_bit_rem
            .assign(region, offset, u16_sign_bit_rem as u32 as u64);
        self.u16_sign_bit_rem_diff
            .assign(region, offset, u16_sign_bit_rem_diff as u32 as u64);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct U64CellWithFlagBitDynSign<F: Field> {
    pub(crate) u64_as_u16_le: [AdviceColumn; 4],
    pub(crate) u64: AdviceColumn,
    pub(crate) sign_bit: AdviceColumn,
    pub(crate) u16_sign_bit_rem: AdviceColumn,
    pub(crate) u16_sign_bit_rem_diff: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> U64CellWithFlagBitDynSign<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
        is_i32_otherwise_i64: bool,
        is_sign: bool,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u64_as_u16_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64.assign(region, offset, value);

        if is_sign {
            let sign_bit_idx = if is_i32_otherwise_i64 { 1 } else { 3 };
            let u16_sign_byte = (value >> (sign_bit_idx * 16)) & 0xffff;
            let sign_bit = u16_sign_byte >> 15;
            let u16_sign_bit_rem = u16_sign_byte & 0x7fff;
            let u16_sign_bit_rem_diff = 0x7fff - u16_sign_bit_rem;
            self.sign_bit.assign(region, offset, sign_bit as u32 as u64);
            self.u16_sign_bit_rem
                .assign(region, offset, u16_sign_bit_rem as u32 as u64);
            self.u16_sign_bit_rem_diff
                .assign(region, offset, u16_sign_bit_rem_diff as u32 as u64);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct U64Cell<F: Field> {
    pub(crate) u64_as_u16_le: [AdviceColumn; 4],
    pub(crate) u64: AdviceColumn,
    _marker: PhantomData<F>,
}

pub(super) fn query_u64_cell<F: Field>(
    cb: &mut OpConstraintBuilder<F>,
    enable: Query<F>,
) -> U64Cell<F> {
    let u64_as_u16_le = cb.query_cells();
    let u64 = cb.query_cell();
    cb.require_zeros(
        "c9. u64 decompose",
        vec![
            (0..4)
                .into_iter()
                .map(|x| u64_as_u16_le[x].current() * Query::from(1u64 << (16 * x)))
                .fold(u64.current(), |acc, x| acc - x)
                * enable,
        ],
    );
    U64Cell {
        u64_as_u16_le,
        u64,
        _marker: Default::default(),
    }
}

pub(crate) fn query_u64_with_flag_bit_cell_dyn<F: Field>(
    cb: &mut OpConstraintBuilder<F>,
    is_i32_otherwise_i64: SelectorColumn,
) -> U64CellWithFlagBitDyn<F> {
    let u64 = query_u64_cell(cb, Query::one());
    let sign_bit = cb.query_cell();
    let u16_sign_bit_rem = cb.query_cell();
    let u16_sign_bit_rem_diff = cb.query_cell();

    cb.require_zeros("sign bit dyn", {
        let flag_u16 = u64.u64_as_u16_le[3].current()
            + is_i32_otherwise_i64.current().0
                * (u64.u64_as_u16_le[1].current() - u64.u64_as_u16_le[3].current());
        vec![
            (sign_bit.current() * Query::from(1 << 15) + u16_sign_bit_rem.current() - flag_u16),
            (u16_sign_bit_rem.current() + u16_sign_bit_rem_diff.current()
                - Query::from((1 << 15) - 1)),
        ]
    });

    U64CellWithFlagBitDyn {
        u64_as_u16_le: u64.u64_as_u16_le,
        u64: u64.u64,
        sign_bit,
        u16_sign_bit_rem,
        u16_sign_bit_rem_diff,
        _marker: Default::default(),
    }
}

impl<F: Field> U64Cell<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u64_as_u16_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64.assign(region, offset, value);
        Ok(())
    }
}
