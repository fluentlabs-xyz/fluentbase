use crate::{
    constraint_builder::{
        AdviceColumn,
        AdviceColumnPhase2,
        BinaryQuery,
        ConstraintBuilder,
        FixedColumn,
        Query,
        SelectorColumn,
        ToExpr,
    },
    lookup_table::{LookupTable, ResponsibleOpcodeLookup, RwLookup, RwasmLookup},
    runtime_circuit::execution_state::ExecutionState,
    state_circuit::tag::RwTableTag,
    trace_step::MAX_STACK_HEIGHT,
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};
use std::ops::{Add, Sub};

#[derive(Clone)]
pub struct StateTransition<F: Field> {
    stack_pointer: AdviceColumn,
    stack_pointer_offset: Query<F>,
    rw_counter: AdviceColumn,
    rw_counter_offset: Query<F>,
}

impl<F: Field> StateTransition<F> {
    pub fn configure(cs: &mut ConstraintSystem<F>) -> Self {
        let stack_pointer = AdviceColumn(cs.advice_column());
        let rw_counter = AdviceColumn(cs.advice_column());
        Self {
            stack_pointer,
            stack_pointer_offset: Query::zero(),
            rw_counter,
            rw_counter_offset: Query::zero(),
        }
    }

    pub fn assign(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        stack_pointer: u64,
        rw_counter: u64,
    ) {
        self.stack_pointer.assign(region, offset, stack_pointer);
        self.stack_pointer.assign(region, offset, rw_counter);
    }

    pub fn stack_pointer(&self) -> Query<F> {
        self.stack_pointer.current() + self.stack_pointer_offset.clone()
    }

    pub fn rw_counter(&self) -> Query<F> {
        self.rw_counter.current() + self.rw_counter_offset.clone()
    }
}

pub struct OpConstraintBuilder<'cs, 'st, F: Field> {
    q_enable: SelectorColumn,
    pub(crate) base: ConstraintBuilder<F>,
    cs: &'cs mut ConstraintSystem<F>,
    // rwasm table fields
    opcode: AdviceColumn,
    value: AdviceColumn,
    index: AdviceColumn,
    // rw fields
    state_transition: &'st mut StateTransition<F>,
    op_lookups: Vec<LookupTable<F>>,
}

use Query as Q;

#[allow(unused_variables)]
impl<'cs, 'st, F: Field> OpConstraintBuilder<'cs, 'st, F> {
    pub fn new(
        cs: &'cs mut ConstraintSystem<F>,
        q_enable: SelectorColumn,
        state_transition: &'st mut StateTransition<F>,
    ) -> Self {
        let opcode = AdviceColumn(cs.advice_column());
        let value = AdviceColumn(cs.advice_column());
        let index = AdviceColumn(cs.advice_column());
        Self {
            q_enable,
            base: ConstraintBuilder::new(q_enable),
            cs,
            opcode,
            value,
            index,
            state_transition,
            op_lookups: vec![],
        }
    }

    pub fn query_rwasm_table(&self) -> [AdviceColumn; 3] {
        [self.index, self.opcode, self.value]
    }

    pub fn query_rwasm_code(&self) -> AdviceColumn {
        self.opcode
    }

    pub fn query_rwasm_value(&self) -> AdviceColumn {
        self.value
    }

    pub fn query_rwasm_index(&self) -> AdviceColumn {
        self.index
    }

    pub fn query_cell(&mut self) -> AdviceColumn {
        self.base.advice_column(self.cs)
    }

    pub fn query_cells<const N: usize>(&mut self) -> [AdviceColumn; N] {
        self.base.advice_columns(self.cs)
    }

    pub fn query_fixed(&mut self) -> FixedColumn {
        self.base.fixed_column(self.cs)
    }

    pub fn query_cell_phase2(&mut self) -> AdviceColumnPhase2 {
        self.base.advice_column_phase2(self.cs)
    }

    pub fn stack_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rw_lookup(is_write, RwTableTag::Stack.expr(), address, value);
    }

    pub fn stack_push(&mut self, value: Query<F>) {
        self.stack_lookup(Query::one(), self.state_transition.stack_pointer(), value);
        self.state_transition.stack_pointer_offset =
            self.state_transition.stack_pointer_offset.clone().sub(1);
    }

    pub fn stack_pop(&mut self, value: Query<F>) {
        self.state_transition.stack_pointer_offset =
            self.state_transition.stack_pointer_offset.clone().add(1);
        self.stack_lookup(Query::zero(), self.state_transition.stack_pointer(), value);
    }

    pub fn global_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rw_lookup(is_write, RwTableTag::Global.expr(), address, value);
    }

    pub fn global_get(&mut self, index: Query<F>, value: Query<F>) {
        self.global_lookup(Query::zero(), index, value);
    }

    pub fn global_set(&mut self, index: Query<F>, value: Query<F>) {
        self.global_lookup(Query::one(), index, value);
    }

    pub fn execution_state_lookup(&mut self, execution_state: ExecutionState, opcode: Query<F>) {
        self.op_lookups.push(LookupTable::ResponsibleOpcode(
            self.base
                .apply_lookup_condition([Query::Constant(F::from(execution_state as u64)), opcode]),
        ));
    }

    pub fn rwasm_lookup(&mut self, index: Query<F>, code: Query<F>, value: Query<F>) {
        self.op_lookups
            .push(LookupTable::Rwasm(self.base.apply_lookup_condition([
                Query::one(),
                index,
                code,
                value,
            ])));
    }

    pub fn table_size(&mut self, table_index: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_fill(&mut self, table_index: Q<F>, start: Q<F>, range: Q<F>, value: Q<F>, size: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_grow(&mut self, table_index: Q<F>, init: Q<F>, grow: Q<F>, res: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_get(&mut self, table_index: Q<F>, elem_index: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_set(&mut self, table_index: Q<F>, elem_index: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_copy(&mut self, table_index: Q<F>, table_index2: Q<F>, elem_index: Q<F>, arg: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_initt(&mut self, table_index: Q<F>, table_index2: Q<F>, elem_index: Q<F>, arg: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }

    pub fn range_check_1024(&mut self, value: Q<F>) {
        // unreachable!("not implemented yet")
    }

    pub fn rw_lookup(
        &mut self,
        is_write: Query<F>,
        tag: Query<F>,
        address: Query<F>,
        value: Query<F>,
    ) {
        self.op_lookups
            .push(LookupTable::Rw(self.base.apply_lookup_condition([
                Query::one(),
                self.state_transition.rw_counter(),
                is_write,
                tag,
                Query::zero(),
                address,
                value,
            ])));
        let condition = self.base.resolve_condition();
        // self.base.condition(condition, |cb| {
        // });
        self.state_transition.rw_counter_offset =
            self.state_transition.rw_counter_offset.clone() + 1;
    }

    pub fn stack_pointer_offset(&self) -> Query<F> {
        Query::from(MAX_STACK_HEIGHT as u64) - self.state_transition.stack_pointer() - 1
    }

    pub fn require_equal(&mut self, name: &'static str, left: Query<F>, right: Query<F>) {
        self.base.assert_zero(name, left - right)
    }

    pub fn require_opcode(&mut self, instr: Instruction) {
        self.require_equal("opcode", self.opcode.current(), instr.code_value().expr());
    }

    pub fn condition(&mut self, condition: Query<F>, configure: impl FnOnce(&mut Self)) {
        self.condition2(BinaryQuery(condition), configure);
    }

    pub fn condition2(&mut self, condition: BinaryQuery<F>, configure: impl FnOnce(&mut Self)) {
        self.base.enter_condition(condition);
        configure(self);
        self.base.leave_condition();
    }

    pub fn build(
        &mut self,
        rwasm_lookup: &impl RwasmLookup<F>,
        rw_lookup: &impl RwLookup<F>,
        responsible_opcode_lookup: &impl ResponsibleOpcodeLookup<F>,
    ) {
        while let Some(state_lookup) = self.op_lookups.pop() {
            match state_lookup {
                LookupTable::Rwasm(fields) => {
                    self.base.add_lookup(
                        "rwasm_lookup(offset,code,value)",
                        fields,
                        rwasm_lookup.lookup_rwasm_table(),
                    );
                }
                LookupTable::Rw(fields) => {
                    self.base.add_lookup(
                        "rw_lookup(rw_counter,is_write,tag,id,address,value)",
                        fields,
                        rw_lookup.lookup_rw_table(),
                    );
                }
                LookupTable::ResponsibleOpcode(fields) => {
                    self.base.add_lookup(
                        "responsible_opcode(execution_state,opcode)",
                        fields,
                        responsible_opcode_lookup.lookup_responsible_opcode_table(),
                    );
                }
            }
        }
        self.base.build(self.cs);
    }
}
