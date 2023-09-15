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

macro_rules! define_cell {
    ($x: ident, $limit: expr) => {
        #[derive(Debug, Clone, Copy)]
        pub(crate) struct $x<F: FieldExt>(pub(crate) AllocatedCell<F>);

        impl<F: FieldExt> CellExpression<F> for $x<F> {
            fn curr_expr(&self, meta: &mut VirtualCells<'_, F>) -> Expression<F> {
                self.0.curr_expr(meta)
            }

            fn assign(
                &self,
                ctx: &mut Context<'_, F>,
                value: F,
            ) -> Result<AssignedCell<F, F>, Error> {
                assert!(
                    value <= $limit,
                    "assigned value {:?} exceeds the limit {:?}",
                    value,
                    $limit
                );

                self.0.assign(ctx, value)
            }
        }
    };
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AllocatedU64CellWithFlagBitDyn<F: Field> {
    pub(crate) u16_cells_le: [AdviceColumn; 4],
    pub(crate) u64_cell: AdviceColumn,
    pub(crate) flag_bit_cell: AdviceColumn,
    pub(crate) flag_u16_rem_cell: AdviceColumn,
    pub(crate) flag_u16_rem_diff_cell: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> AllocatedU64CellWithFlagBitDyn<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
        is_i32: bool,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u16_cells_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64_cell.assign(region, offset, value);

        let pos = if is_i32 { 1 } else { 3 };
        let u16_value = (value >> (pos * 16)) & 0xffff;
        let u16_flag_bit = u16_value >> 15;
        let u16_rem = u16_value & 0x7fff;
        let u16_rem_diff = 0x7fff - u16_rem;
        self.flag_bit_cell
            .assign(region, offset, u16_flag_bit as u32 as u64);
        self.flag_u16_rem_cell
            .assign(region, offset, u16_rem as u32 as u64);
        self.flag_u16_rem_diff_cell
            .assign(region, offset, u16_rem_diff as u32 as u64);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AllocatedU64CellWithFlagBitDynSign<F: Field> {
    pub(crate) u16_cells_le: [AdviceColumn; 4],
    pub(crate) u64_cell: AdviceColumn,
    pub(crate) flag_bit_cell: AdviceColumn,
    pub(crate) flag_u16_rem_cell: AdviceColumn,
    pub(crate) flag_u16_rem_diff_cell: AdviceColumn,
    _marker: PhantomData<F>,
}

impl<F: Field> AllocatedU64CellWithFlagBitDynSign<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
        is_i32: bool,
        is_sign: bool,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u16_cells_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64_cell.assign(region, offset, value);

        if is_sign {
            let pos = if is_i32 { 1 } else { 3 };
            let u16_value = (value >> (pos * 16)) & 0xffff;
            let u16_flag_bit = u16_value >> 15;
            let u16_rem = u16_value & 0x7fff;
            let u16_rem_diff = 0x7fff - u16_rem;
            self.flag_bit_cell
                .assign(region, offset, u16_flag_bit as u32 as u64);
            self.flag_u16_rem_cell
                .assign(region, offset, u16_rem as u32 as u64);
            self.flag_u16_rem_diff_cell
                .assign(region, offset, u16_rem_diff as u32 as u64);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AllocatedU64Cell<F: Field> {
    pub(crate) u16_cells_le: [AdviceColumn; 4],
    pub(crate) u64_cell: AdviceColumn,
    _marker: PhantomData<F>,
}

pub(super) fn prepare_alloc_u64_cell<F: Field>(
    cb: &mut OpConstraintBuilder<F>,
    enable: Query<F>,
) -> AllocatedU64Cell<F> {
    let u16_cells_le = cb.query_cells();
    let u64_cell = cb.query_cell();
    // meta.create_gate("c9. u64 decompose", |meta| {
    let init = u64_cell.current();
    cb.require_zeros(
        "c9. u64 decompose",
        vec![
            (0..4)
                .into_iter()
                .map(|x| u16_cells_le[x].current() * Query::from(1u64 << (16 * x)))
                .fold(init, |acc, x| acc - x)
                * enable,
        ],
    );
    // });
    AllocatedU64Cell {
        u16_cells_le,
        u64_cell,
        _marker: Default::default(),
    }
}

pub(crate) fn alloc_u64_with_flag_bit_cell_dyn<F: Field>(
    cb: &mut OpConstraintBuilder<F>,
    is_i32: SelectorColumn,
) -> AllocatedU64CellWithFlagBitDyn<F> {
    let value = prepare_alloc_u64_cell(cb, Query::one());
    let flag_bit_cell = cb.query_cell();
    let flag_u16_rem_cell = cb.query_cell();
    let flag_u16_rem_diff_cell = cb.query_cell();

    cb.require_zeros("flag bit dyn", {
        let flag_u16 = value.u16_cells_le[3].current()
            + is_i32.current().0
                * (value.u16_cells_le[1].current() - value.u16_cells_le[3].current());
        vec![
            (flag_bit_cell.current() * Query::from(1 << 15) + flag_u16_rem_cell.current()
                - flag_u16),
            (flag_u16_rem_cell.current() + flag_u16_rem_diff_cell.current()
                - Query::from((1 << 15) - 1)),
        ]
    });

    AllocatedU64CellWithFlagBitDyn {
        u16_cells_le: value.u16_cells_le,
        u64_cell: value.u64_cell,
        flag_bit_cell,
        flag_u16_rem_cell,
        flag_u16_rem_diff_cell,
        _marker: Default::default(),
    }
}

impl<F: Field> AllocatedU64Cell<F> {
    pub(crate) fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        value: u64,
    ) -> Result<(), GadgetError> {
        for i in 0..4 {
            self.u16_cells_le[i].assign(region, offset, (value >> (i * 16)) & 0xffffu64);
        }
        self.u64_cell.assign(region, offset, value);
        Ok(())
    }
}
