use crate::{
    constraint_builder::{AdviceColumn, Query, ToExpr},
    exec_step::ExecStep,
    gadgets::lt::LtGadget,
    runtime_circuit::constraint_builder::OpConstraintBuilder,
    util::Field,
};
use fluentbase_rwasm::rwasm::N_BYTES_PER_MEMORY_PAGE;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub struct MemoryExpansionGadget<F: Field> {
    used_pages: AdviceColumn,
    used_memory: AdviceColumn,
    value: AdviceColumn,
    lt_gadget: LtGadget<F, 8>,
    marker: PhantomData<F>,
}

impl<'cs, 'st, 'dcm, F: Field> MemoryExpansionGadget<F> {
    pub fn configure(cb: &mut OpConstraintBuilder<'cs, 'st, 'dcm, F>) -> Self {
        let used_pages = cb.query_cell();
        let used_memory = cb.query_cell();
        let value = cb.query_cell();
        // cb.context_lookup(
        //     RwTableContextTag::MemorySize,
        //     0.expr(),
        //     used_memory.current(),
        //     used_memory.current(),
        // );
        let max_cap: Query<F> = used_pages.current() * N_BYTES_PER_MEMORY_PAGE.expr();
        let lt_gadget = cb.lt_gadget(max_cap, used_memory.current());
        cb.condition(lt_gadget.expr(), |_cb| {});
        Self {
            used_pages,
            used_memory,
            value,
            lt_gadget,
            marker: Default::default(),
        }
    }

    pub fn assign(&self, region: &mut Region<'_, F>, offset: usize, step: &ExecStep) {
        let used_memory = step.curr().memory_size as u64;
        let used_pages = ((step.curr().memory_size + N_BYTES_PER_MEMORY_PAGE - 1)
            / N_BYTES_PER_MEMORY_PAGE) as u64;
        self.used_pages.assign(region, offset, used_pages);
        self.used_memory.assign(region, offset, used_memory);
        self.lt_gadget.assign(
            region,
            offset,
            F::from(used_pages * N_BYTES_PER_MEMORY_PAGE as u64),
            F::from(used_memory),
        );
    }
}
