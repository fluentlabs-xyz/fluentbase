use crate::{
    constraint_builder::{AdviceColumn, ConstraintBuilder, Query},
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

    pub fn build_start_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 1.0. Unused keys are 0
        // cb.assert_equal("address is 0 for Start", self.address.current());
        // cb.assert_equal("id is 0 for Start", self.id.current());
        // 1.1. rw_counter increases by 1 for every non-first row
        // cb.require_zero(
        //     "rw_counter increases by 1 for every non-first row",
        //     q.lexicographic_ordering_selector.clone() * (q.rw_counter_change() - 1.expr()),
        // );
        // 1.2. Start value is 0
        // cb.assert_equal("Start value is 0", self.value.current());
        // 1.3. Start initial value is 0
        // 1.4. state_root is unchanged for every non-first row
        // cb.condition(q.lexicographic_ordering_selector.clone(), |cb| {
        //     cb.require_equal(
        //         "state_root is unchanged for Start",
        //         q.state_root(),
        //         q.state_root_prev(),
        //     )
        // });
        // cb.assert_equal(
        //     "value_prev column is 0 for Start",
        //     self.value_prev.current(),
        // );
    }

    pub fn build_memory_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 2.0. Unused keys are 0
        // 2.1. First access for a set of all keys are 0 if READ
        // cb.require_zero(
        //     "first access for a set of all keys are 0 if READ",
        //     q.first_access() * q.is_read() * q.value(),
        // );
        // could do this more efficiently by just asserting address = limb0 + 2^16 *
        // limb1?
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
        // cb.require_zero("initial Memory value is 0", q.initial_value());
        // 2.5. state root does not change
        // cb.require_equal(
        //     "state_root is unchanged for Memory",
        //     q.state_root(),
        //     q.state_root_prev(),
        // );
        // cb.require_equal(
        //     "value_prev column equals initial_value for Memory",
        //     q.value_prev_column(),
        //     q.initial_value(),
        // );
    }

    pub fn build_stack_constraints(&self, cb: &mut ConstraintBuilder<F>) {
        // 3.0. Unused keys are 0
        // 3.1. First access for a set of all keys
        // cb.require_zero(
        //     "first access to new stack address is a write",
        //     q.first_access() * (1.expr() - q.is_write()),
        // );
        // 3.2. stack_ptr in range
        // cb.add_lookup(
        //     "stack address fits into 10 bits",
        //     vec![(q.rw_table.address.clone(), q.lookups.u10.clone())],
        // );
        // 3.3. stack_ptr only increases by 0 or 1
        // cb.condition(q.is_tag_and_id_unchanged.clone(), |cb| {
        //     cb.require_boolean(
        //         "if previous row is also Stack with unchanged call id, address change is 0 or 1",
        //         q.address_change(),
        //     )
        // });
        // 3.4. Stack initial value is 0
        // 3.5 state root does not change
        // cb.require_equal(
        //     "state_root is unchanged for Stack",
        //     q.state_root(),
        //     q.state_root_prev(),
        // );
        // cb.require_equal(
        //     "value_prev column equals initial_value for Stack",
        //     q.value_prev_column(),
        //     q.initial_value(),
        // );
    }

    pub fn build_global_constraints(&self, cb: &mut ConstraintBuilder<F>) {}

    pub fn build_table_constraints(&self, cb: &mut ConstraintBuilder<F>) {}
}
