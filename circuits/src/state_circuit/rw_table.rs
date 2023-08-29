use crate::{
    constraint_builder::{AdviceColumn, BinaryQuery, ConstraintBuilder, Query, ToExpr},
    util::Field,
};
use halo2_proofs::plonk::ConstraintSystem;
use std::marker::PhantomData;

const N_STATE_LOOKUP_TABLE: usize = 8;

pub trait RwLookup<F: Field> {
    fn lookup_rw_table(&self) -> [Query<F>; N_STATE_LOOKUP_TABLE];
}

#[derive(Clone)]
pub struct RwTable<F: Field> {
    pub(crate) q_enable: AdviceColumn,
    pub(crate) rw_counter: AdviceColumn,
    pub(crate) is_write: AdviceColumn,
    pub(crate) tag: AdviceColumn,
    pub(crate) id: AdviceColumn,
    pub(crate) address: AdviceColumn,
    pub(crate) value: AdviceColumn,
    pub(crate) value_prev: AdviceColumn,
    // additional fields
    not_first_access: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> RwLookup<F> for RwTable<F> {
    fn lookup_rw_table(&self) -> [Query<F>; N_STATE_LOOKUP_TABLE] {
        [
            self.q_enable.current(),
            self.rw_counter.current(),
            self.is_write.current(),
            self.tag.current(),
            self.id.current(),
            self.address.current(),
            self.value.current(),
            self.value_prev.current(),
        ]
    }
}

impl<F: Field> RwTable<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        Self {
            q_enable: AdviceColumn(cs.advice_column()),
            rw_counter: AdviceColumn(cs.advice_column()),
            is_write: AdviceColumn(cs.advice_column()),
            tag: AdviceColumn(cs.advice_column()),
            id: AdviceColumn(cs.advice_column()),
            address: AdviceColumn(cs.advice_column()),
            value: AdviceColumn(cs.advice_column()),
            value_prev: AdviceColumn(cs.advice_column()),
            not_first_access: AdviceColumn(cs.advice_column()),
            marker: Default::default(),
        }
    }

    fn q_first_access(&self) -> BinaryQuery<F> {
        !BinaryQuery(self.not_first_access.current())
    }

    fn q_is_write(&self) -> BinaryQuery<F> {
        BinaryQuery(self.is_write.current())
    }

    fn q_is_read(&self) -> BinaryQuery<F> {
        !BinaryQuery(self.is_write.current())
    }

    fn q_is_tag_and_id_unchanged(&self) -> BinaryQuery<F> {
        let tag_unchanged = !BinaryQuery(self.tag.current() - self.tag.previous());
        let id_unchanged = !BinaryQuery(self.id.current() - self.id.previous());
        tag_unchanged.and(id_unchanged)
    }

    fn q_address_change(&self) -> Query<F> {
        self.address.current() - self.address.previous()
    }

    pub fn build_general_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // tag value in RwTableTag range is enforced in BinaryNumberChip
        cb.assert_boolean("is_write is boolean", self.is_write.current());

        // 1 if first_different_limb is in the rw counter, 0 otherwise (i.e. any of the
        // 4 most significant bits are 0)
        // cb.require_equal(
        //     "not_first_access when first 16 limbs are same",
        //     q.not_first_access.clone(),
        //     q.first_different_limb[0].clone()
        //         * q.first_different_limb[1].clone()
        //         * q.first_different_limb[2].clone()
        //         * q.first_different_limb[3].clone(),
        // );

        // When at least one of the keys (tag, id, address, field_tag, or storage_key)
        // in the current row differs from the previous row.
        cb.condition(self.q_first_access().and(self.q_is_read()), |cb| {
            cb.assert_zero(
                "first access reads don't change value",
                self.value.current(),
            );
        });

        // When all the keys in the current row and previous row are equal.
        cb.condition(!self.q_first_access(), |cb| {
            cb.assert_zero(
                "non-first access reads don't change value",
                (1.expr() - self.is_write.current())
                    * (self.value.current() - self.value_prev.current()),
            );
        });
    }

    pub fn build_start_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 1.0. Unused keys are 0
        cb.assert_zero("address is 0 for Start", self.address.current());
        cb.assert_zero("id is 0 for Start", self.id.current());
        // 1.1. rw_counter increases by 1 for every non-first row
        // cb.assert_zero(
        //     "rw_counter increases by 1 for every non-first row",
        //     q.lexicographic_ordering_selector.clone() * (q.rw_counter_change() - 1.expr()),
        // );
        // 1.2. Start value is 0
        cb.assert_zero("Start value is 0", self.value.current());
        // 1.3. Start initial value is 0
        // 1.4. state_root is unchanged for every non-first row
        cb.assert_zero(
            "value_prev column is 0 for Start",
            self.value_prev.current(),
        );
    }

    pub fn build_memory_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 2.0. Unused keys are 0
        // 2.1. First access for a set of all keys are 0 if READ
        cb.condition(self.q_first_access().and(self.q_is_read()), |cb| {
            cb.assert_zero(
                "first access for a set of all keys are 0 if READ",
                self.value.current(),
            );
        });
        // 2.2. mem_addr in range
        // for limb in &q.address.limbs[2..] {
        //     cb.require_zero("memory address fits into 2 limbs", limb.clone());
        // }
        // 2.3. value is a byte
        // cb.add_lookup(
        //     "memory value is a byte",
        //     vec![(q.rw_table.value.clone(), q.lookups.u8.clone())],
        // );
        // 2.4. Start initial value is 0
        // 2.5. state root does not change
    }

    pub fn build_stack_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 3.0. Unused keys are 0
        // 3.1. First access for a set of all keys
        cb.condition(self.q_first_access(), |cb| {
            cb.assert_zero(
                "first access to new stack address is a write",
                1.expr() - self.is_write.current(),
            );
        });
        // 3.2. stack_ptr in range
        // cb.add_lookup(
        //     "stack address fits into 10 bits",
        //     vec![(q.rw_table.address.clone(), q.lookups.u10.clone())],
        // );
        // 3.3. stack_ptr only increases by 0 or 1
        cb.condition(self.q_is_tag_and_id_unchanged(), |cb| {
            cb.assert_boolean(
                "if previous row is also Stack with unchanged call id, address change is 0 or 1",
                self.address.current() - self.address.previous(),
            )
        });
        // 3.4. Stack initial value is 0
        // 3.5 state root does not change
    }

    pub fn build_global_constraints(&self, _cb: &mut ConstraintBuilder<F>) {}

    pub fn build_table_constraints(&self, _cb: &mut ConstraintBuilder<F>) {}
}
