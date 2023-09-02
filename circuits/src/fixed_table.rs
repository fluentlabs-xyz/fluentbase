use crate::{
    constraint_builder::{FixedColumn, Query, ToExpr},
    impl_expr,
    lookup_table::{FixedLookup, N_FIXED_LOOKUP_TABLE},
    util::Field,
};
use halo2_proofs::{
    circuit::Layouter,
    plonk::{ConstraintSystem, Error},
};
use itertools::Itertools;
use std::marker::PhantomData;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[derive(Copy, Clone, EnumIter)]
pub enum FixedTableTag {
    Zero,
    Range5,
    Range16,
    Range32,
    Range64,
    Range128,
    Range256,
    Range256x2,
    Range512,
    Range1024,
    SignByte,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    Pow2,
    OpRel,
    PopCnt,
    Clz,
    ClzFilter,
    Ctz,
    CzOut,
}

impl_expr!(FixedTableTag);

impl FixedTableTag {
    pub fn build<F: Field>(&self) -> Box<dyn Iterator<Item = [F; 4]>> {
        let tag = F::from(*self as u64);
        match self {
            Self::Zero => Box::new((0..1).map(move |_| [tag, F::zero(), F::zero(), F::zero()])),
            Self::Range5 => {
                Box::new((0..5).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range16 => {
                Box::new((0..16).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range32 => {
                Box::new((0..32).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range64 => {
                Box::new((0..64).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range128 => {
                Box::new((0..128).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range256 => {
                Box::new((0..256).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range256x2 => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| [tag, F::from(lhs), F::from(rhs), F::zero()])
            })),
            Self::Range512 => {
                Box::new((0..512).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::Range1024 => {
                Box::new((0..1024).map(move |value| [tag, F::from(value), F::zero(), F::zero()]))
            }
            Self::SignByte => Box::new((0..256).map(move |value| {
                [
                    tag,
                    F::from(value),
                    F::from((value >> 7) * 0xFFu64),
                    F::zero(),
                ]
            })),
            Self::BitwiseAnd => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| [tag, F::from(lhs), F::from(rhs), F::from(lhs & rhs)])
            })),
            Self::BitwiseOr => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| [tag, F::from(lhs), F::from(rhs), F::from(lhs | rhs)])
            })),
            Self::BitwiseXor => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| [tag, F::from(lhs), F::from(rhs), F::from(lhs ^ rhs)])
            })),
            Self::Pow2 => Box::new((0..256).map(move |value| {
                let (pow_lo, pow_hi) = if value < 128 {
                    (F::from_u128(1_u128 << value), F::from(0))
                } else {
                    (F::from(0), F::from_u128(1 << (value - 128)))
                };
                [tag, F::from(value), pow_lo, pow_hi]
            })),
            Self::PopCnt => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| {
                    [
                        tag,
                        F::from(lhs),
                        F::from(rhs),
                        F::from(bitintr::Popcnt::popcnt(lhs | rhs << 8)),
                    ]
                })
            })),
            Self::OpRel => Box::new((0..256).flat_map(move |lhs| {
                // OpRel encoding: Neq: 0, Eq: 1, Gt: 2, Ge: 3, Lt: 4, Le: 5
                // Code part will be constructed from verified bits, so rhs is correct to check by
                // fix table.
                (0..(256 * 6)).map(move |rhs_and_code| {
                    let rhs = rhs_and_code & 0xff;
                    let code = rhs_and_code >> 8;
                    let out = match code {
                        0 => lhs != rhs,
                        1 => lhs == rhs,
                        2 => lhs > rhs,
                        3 => lhs >= rhs,
                        4 => lhs < rhs,
                        5 => lhs <= rhs,
                        _ => unreachable!(),
                    };
                    [
                        tag,
                        F::from(lhs),
                        F::from(rhs_and_code),
                        F::from(out as u64),
                    ]
                })
            })),
            Self::Clz => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| {
                    [
                        tag,
                        F::from(lhs),
                        F::from(rhs),
                        F::from(bitintr::Lzcnt::lzcnt((lhs | rhs << 8) as u16) as u64),
                    ]
                })
            })),
            // Lhs argument is what to count, about leading zeros.
            // Rhs is source to get one bit at position of last one bit from lhs.
            // Result is this filtered bit.
            Self::ClzFilter => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| {
                    let lzcnt = bitintr::Lzcnt::lzcnt(lhs as u8) as u64;
                    let pos = 7 - lzcnt.min(7);
                    let bit = 1 << pos;
                    let filtred = (rhs & bit) >> pos;
                    [tag, F::from(lhs), F::from(rhs), F::from(filtred)]
                })
            })),
            Self::Ctz => Box::new((0..256).flat_map(move |lhs| {
                (0..256).map(move |rhs| {
                    [
                        tag,
                        F::from(lhs),
                        F::from(rhs),
                        F::from(bitintr::Tzcnt::tzcnt((lhs | rhs << 8) as u16) as u64),
                    ]
                })
            })),
            Self::CzOut => Box::new((0..289).flat_map(move |lhs| {
                // Logic is to accumulate when it equal to 16, otherwize summ and return.
                // If arguments is all zero, than result is zero.
                (0..289).map(move |rhs| {
                    [
                        tag,
                        F::from(lhs),
                        F::from(rhs),
                        F::from({
                            let list = [lhs % 17, lhs / 17, rhs % 17, rhs / 17];
                            let mut out = 0;
                            for i in 0..4 {
                                if list[i] == 16 {
                                    out += 16;
                                } else {
                                    out += list[i];
                                    break;
                                }
                            }
                            out
                        }),
                    ]
                })
            })),
        }
    }
}

#[derive(Clone)]
pub struct FixedTable<F: Field> {
    fixed_table: [FixedColumn; 4],
    pd: PhantomData<F>,
}

impl<F: Field> FixedTable<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            fixed_table: [
                FixedColumn(cs.fixed_column()),
                FixedColumn(cs.fixed_column()),
                FixedColumn(cs.fixed_column()),
                FixedColumn(cs.fixed_column()),
            ],
            pd: Default::default(),
        }
    }

    pub fn load(&self, layouter: &mut impl Layouter<F>) -> Result<(), Error> {
        layouter.assign_region(
            || "fixed table",
            |mut region| {
                for (offset, row) in std::iter::once([F::zero(); 4])
                    .chain(FixedTableTag::iter().flat_map(|tag| tag.build()))
                    .enumerate()
                {
                    for (column, value) in self.fixed_table.iter().zip_eq(row) {
                        column.assign(&mut region, offset, value);
                    }
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

impl<F: Field> FixedLookup<F> for FixedTable<F> {
    fn lookup_fixed_table(&self) -> [Query<F>; N_FIXED_LOOKUP_TABLE] {
        [
            self.fixed_table[0].current(),
            self.fixed_table[1].current(),
            self.fixed_table[2].current(),
            self.fixed_table[3].current(),
        ]
    }
}
