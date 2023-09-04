use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query, SelectorColumn, ToExpr},
    gadgets::binary_number::{BinaryNumberChip, BinaryNumberConfig},
    impl_expr,
    lookup_table::RangeCheckLookup,
    state_circuit::{
        param::{N_LIMBS_ADDRESS, N_LIMBS_ID, N_LIMBS_RW_COUNTER},
        rw_row::RwRow,
        sort_keys::SortKeysConfig,
    },
    util::Field,
};
use halo2_proofs::{
    circuit::Region,
    plonk::{ConstraintSystem, Error},
    poly::Rotation,
};
use itertools::Itertools;
use std::iter::once;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

// We use this chip to show that the rows of the rw table are in lexicographic
// order, i.e. ordered by (tag, id, address, field_tag, storage_key, and
// rw_counter). We do this by packing these 6 fields into a 512 bit value X, and
// then showing that X_cur > X_prev. Let A0, A1, ..., A31 be the 32 16-bit limbs
// of X_cur and B0, B1, ..., B31 be 32 16-bit limbs of X_prev, in big endian
// order.

// Let
// C0 = A0 - B0,
// C1 = C0 << 16 + A1 - B1,
// ...
// C31 = C30 << 16 + A31 - B21.

// X_cur > X_prev iff one of C0, ..., C31 is non-zdero and fits into 16 bits.
// C16, ..., C31 do not necessarily fit into a field element, so to check that
// Cn fits into 16 bits, we use an RLC to check that Cn-1 = 0 and then do a
// lookup for An-Bn.

// We show this with following advice columns and constraints:
// - first_different_limb: first index where the limbs differ. We use a BinaryNumberChip here to
//   reduce the degree of the constraints.
// - limb_difference: the difference between the limbs at first_different_limb.
// - limb_difference_inverse: the inverse of limb_difference

//  1. limb_difference fits into 16 bits.
//  2. limb_difference is not zero because its inverse exists.
//  3. RLC of the pairwise limb differences before the first_different_limb is zero.
//  4. limb_difference equals the difference of the limbs at first_different_limb.

#[derive(Clone, Copy, Debug, EnumIter)]
pub enum LimbIndex {
    Tag,
    Id1,
    Id0,
    Address1,
    Address0,
    RwCounter1,
    RwCounter0,
}

impl_expr!(LimbIndex);

impl Into<usize> for LimbIndex {
    fn into(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy)]
pub struct LexicographicOrderingConfig {
    pub(crate) selector: SelectorColumn,
    pub first_different_limb: BinaryNumberConfig<LimbIndex, 5>,
    limb_difference: AdviceColumn,
    limb_difference_inverse: AdviceColumn,
}

impl LexicographicOrderingConfig {
    pub fn configure<F: Field>(
        cs: &mut ConstraintSystem<F>,
        keys: &SortKeysConfig<F>,
        _range_check_lookup: &impl RangeCheckLookup<F>,
    ) -> Self {
        let selector = SelectorColumn(cs.fixed_column());
        let first_different_limb = BinaryNumberChip::configure(cs, selector, None);
        let limb_difference = AdviceColumn(cs.advice_column());
        let limb_difference_inverse = AdviceColumn(cs.advice_column());

        let mut cb = ConstraintBuilder::new(selector);

        let cur = Queries::new(keys, Rotation::cur());
        let prev = Queries::new(keys, Rotation::prev());

        let config = LexicographicOrderingConfig {
            selector,
            first_different_limb,
            limb_difference,
            limb_difference_inverse,
        };

        // cb.add_lookup(
        //     "limb_difference fits into u16",
        //     [limb_difference.current()],
        //     range_check_lookup.lookup_u16_table(),
        // );

        cb.assert_zero(
            "limb_difference is not zero",
            Query::one() - limb_difference.current() * limb_difference_inverse.current(),
        );

        let base = Query::from(0x10000);
        for (i, expr) in LimbIndex::iter().zip(calc_limb_differences(&cur, &prev, base)) {
            cb.assert_zero(
                "limb differences before first_different_limb are all 0",
                first_different_limb.value_equals(i, Rotation::cur()).0 * expr,
            )
        }

        for ((i, cur_limb), prev_limb) in LimbIndex::iter().zip(cur.be_limbs()).zip(prev.be_limbs())
        {
            cb.assert_zero(
                "limb_difference equals difference of limbs at index",
                first_different_limb.value_equals(i, Rotation::cur()).0
                    * (limb_difference.current() - cur_limb + prev_limb),
            );
        }

        cb.build(cs);

        config
    }

    // Returns true if the `cur` row is a first access to a group (at least one of
    // tag, id, address, field_tag, or storage_key is different from the one in
    // `prev`), and false otherwise.
    pub fn assign<F: Field>(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        cur: &RwRow,
        prev: &RwRow,
    ) -> Result<LimbIndex, Error> {
        self.selector.enable(region, offset);

        let cur_be_limbs = rw_to_be_limbs(cur);
        let prev_be_limbs = rw_to_be_limbs(prev);

        let find_result = LimbIndex::iter()
            .zip(&cur_be_limbs)
            .zip(&prev_be_limbs)
            .find(|((_, a), b)| a != b);
        let ((index, cur_limb), prev_limb) = if cfg!(test) {
            find_result.unwrap_or(((LimbIndex::RwCounter0, &0), &0))
        } else {
            find_result.expect("repeated rw counter")
        };

        BinaryNumberChip::construct(self.first_different_limb).assign(region, offset, &index)?;

        let limb_difference = F::from(*cur_limb as u64) - F::from(*prev_limb as u64);
        self.limb_difference.assign(region, offset, limb_difference);
        self.limb_difference_inverse
            .assign(region, offset, limb_difference.invert().unwrap());

        Ok(index)
    }

    /// Annotates columns of this gadget embedded within a circuit region.
    pub fn annotate_columns_in_region<F: Field>(&self, region: &mut Region<F>, prefix: &str) {
        [
            (self.limb_difference, "LO_limb_difference"),
            (self.limb_difference_inverse, "LO_limb_difference_inverse"),
        ]
        .iter()
        .for_each(|(col, ann)| region.name_column(|| format!("{}_{}", prefix, ann), *col));
        // fixed column
        region.name_column(
            || format!("{}_LO_upper_limb_difference", prefix),
            self.selector.0,
        );
    }
}

struct Queries<F: Field> {
    tag: Query<F>, // 4 bits
    id_limbs: [Query<F>; N_LIMBS_ID],
    address_limbs: [Query<F>; N_LIMBS_ADDRESS],
    rw_counter_limbs: [Query<F>; N_LIMBS_RW_COUNTER],
}

impl<F: Field> Queries<F> {
    fn new(keys: &SortKeysConfig<F>, rotation: Rotation) -> Self {
        let tag = keys.tag.value(rotation);
        let mut query_advice = |column: AdviceColumn| column.rotation(rotation.0);
        Self {
            tag,
            id_limbs: keys.id.limbs.map(&mut query_advice),
            address_limbs: keys.address.limbs.map(&mut query_advice),
            rw_counter_limbs: keys.rw_counter.limbs.map(query_advice),
        }
    }

    fn be_limbs(&self) -> Vec<Query<F>> {
        once(&self.tag)
            .chain(self.id_limbs.iter().rev())
            .chain(self.address_limbs.iter().rev())
            .chain(self.rw_counter_limbs.iter().rev())
            .cloned()
            .collect()
    }
}

fn rw_to_be_limbs(row: &RwRow) -> Vec<u16> {
    let mut be_bytes = vec![0u8];
    be_bytes.push(row.tag() as u8);
    be_bytes.extend_from_slice(&(row.id().unwrap_or_default() as u32).to_be_bytes());
    be_bytes.extend_from_slice(&(row.address().unwrap_or_default().to_be_bytes()));
    be_bytes.extend_from_slice(&((row.rw_counter() as u32).to_be_bytes()));
    be_bytes
        .iter()
        .tuples()
        .map(|(hi, lo)| u16::from_be_bytes([*hi, *lo]))
        .collect()
}

// Returns a vector of length 32 with the rlc of the limb differences between
// from 0 to i-l. 0 for i=0,
fn calc_limb_differences<F: Field>(
    cur: &Queries<F>,
    prev: &Queries<F>,
    base: Query<F>,
) -> Vec<Query<F>> {
    let mut result = vec![];
    let mut partial_sum = Query::<F>::zero();
    for (cur_limb, prev_limb) in cur.be_limbs().iter().zip(&prev.be_limbs()) {
        result.push(partial_sum.clone());
        partial_sum = partial_sum + base.clone() * (cur_limb.clone() - prev_limb.clone());
    }
    result
}

#[cfg(test)]
mod test {
    use super::LimbIndex;
    use crate::gadgets::binary_number::{from_bits_be, AsBits};
    use strum::IntoEnumIterator;

    #[test]
    fn enough_bits_for_limb_index() {
        for index in LimbIndex::iter() {
            assert_eq!(
                from_bits_be(&<LimbIndex as AsBits<14>>::as_bits(&index)) as u32,
                index as u32
            );
        }
    }
}
