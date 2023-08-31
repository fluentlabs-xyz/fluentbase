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
    runtime_circuit::execution_state::ExecutionState,
    rwasm_circuit::RwasmLookup,
    state_circuit::{tag::RwTableTag, StateLookup},
    trace_step::MAX_STACK_HEIGHT,
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::plonk::ConstraintSystem;
use std::ops::{Add, Sub};

pub struct OpStateTransition {
    stack_pointer: AdviceColumn,
}

pub struct OpConstraintBuilder<'cs, F: Field> {
    q_enable: SelectorColumn,
    pub(crate) base: ConstraintBuilder<F>,
    cs: &'cs mut ConstraintSystem<F>,
    // rwasm table fields
    opcode: AdviceColumn,
    value: AdviceColumn,
    index: AdviceColumn,
    // rw fields
    stack_pointer: Query<F>,
    rw_counter: Query<F>,

    rw_lookups: Vec<[Query<F>; 4]>,
}

#[allow(unused_variables)]
impl<'cs, F: Field> OpConstraintBuilder<'cs, F> {
    pub fn new(cs: &'cs mut ConstraintSystem<F>, q_enable: SelectorColumn) -> Self {
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
            stack_pointer: Query::from(MAX_STACK_HEIGHT as u64 - 1),
            rw_counter: Query::from(0u64),
            rw_lookups: vec![],
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
        self.stack_lookup(Query::one(), self.stack_pointer.clone(), value);
        self.stack_pointer = self.base.resolve_condition().0 * self.stack_pointer.clone().sub(1);
    }

    pub fn stack_pop(&mut self, value: Query<F>) {
        self.stack_pointer = self.base.resolve_condition().0 * self.stack_pointer.clone().add(1);
        self.stack_lookup(Query::zero(), self.stack_pointer.clone(), value);
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

    pub fn execution_state_lookup(&mut self, execution_state: ExecutionState) {
        // unreachable!("not implemented yet")
    }

    pub fn rwasm_lookup(
        &mut self,
        q_enable: BinaryQuery<F>,
        index: Query<F>,
        code: Query<F>,
        value: Query<F>,
        rwasm_lookup: &impl RwasmLookup<F>,
    ) {
        self.base.add_lookup(
            "rwasm_lookup(offset,code,value)",
            [q_enable.0, index, code, value],
            rwasm_lookup.lookup_rwasm_table(),
        );
    }

    pub fn rw_lookup(
        &mut self,
        is_write: Query<F>,
        tag: Query<F>,
        address: Query<F>,
        value: Query<F>,
    ) {
        self.rw_lookups.push([
            self.q_enable.current().0,
            self.rw_counter.clone(),
            is_write,
            tag,
            // Query::zero(),
            // address,
            // value,
        ]);
        self.rw_counter = self.base.resolve_condition().0 * self.rw_counter.clone().add(1);
    }

    pub fn poseidon_lookup(&mut self) {
        // unreachable!("not implemented yet")
    }

    pub fn stack_pointer_offset(&self) -> Query<F> {
        Query::from(MAX_STACK_HEIGHT as u64) - self.stack_pointer.clone() - 1
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

    pub fn build(&mut self, lookup: &impl StateLookup<F>) {
        while let Some(state_lookup) = self.rw_lookups.pop() {
            self.base.add_lookup(
                "rwtable_lookup(q_enable,rw_counter,is_write,tag,id,address,value,value_prev)",
                state_lookup,
                lookup.lookup_rwtable(),
            );
        }
        self.base.build(self.cs);
    }
}
