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

pub struct OpStateTransition {
    stack_pointer: AdviceColumn,
}

pub struct OpConstraintBuilder<'cs, F: Field> {
    q_enable: SelectorColumn,
    pub(crate) base: ConstraintBuilder<F>,
    cs: &'cs mut ConstraintSystem<F>,
    opcode: AdviceColumn,
    value: AdviceColumn,
    index: AdviceColumn,

    rwtable_lookups: Vec<[Query<F>; 8]>,
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
            rwtable_lookups: vec![],
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
        self.rwtable_lookup(
            BinaryQuery::one(),
            self.index.current() + 1,
            is_write,
            RwTableTag::Stack.expr(),
            Query::zero(),
            address,
            value,
            Query::zero(),
        );
    }
    pub fn stack_push(&mut self, value: Query<F>) {
        self.stack_lookup(
            Query::one(),
            Query::from(MAX_STACK_HEIGHT as u64) - self.index.current(),
            value,
        );
    }
    pub fn stack_pop(&mut self, value: Query<F>) {
        self.stack_lookup(
            Query::zero(),
            Query::from(MAX_STACK_HEIGHT as u64) - self.index.current(),
            value,
        );
    }
    pub fn global_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rwtable_lookup(
            BinaryQuery::one(),
            self.index.current() + 1,
            is_write,
            RwTableTag::Global.expr(),
            Query::zero(),
            address,
            value,
            Query::zero(),
        );
    }

    pub fn global_get(&mut self, index: Query<F>, value: Query<F>) {
        self.stack_lookup(Query::zero(), self.index.current(), value);
    }
    pub fn global_set(&mut self, index: Query<F>, value: Query<F>) {
        self.stack_lookup(Query::one(), self.index.current(), value);
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

    pub fn rwtable_lookup(
        &mut self,
        q_enable: BinaryQuery<F>,
        rw_counter: Query<F>,
        is_write: Query<F>,
        tag: Query<F>,
        id: Query<F>,
        address: Query<F>,
        value: Query<F>,
        value_prev: Query<F>,
    ) {
        self.rwtable_lookups.push([
            q_enable.0, rw_counter, is_write, tag, id, address, value, value_prev,
        ]);
    }

    pub fn poseidon_lookup(&mut self) {
        // unreachable!("not implemented yet")
    }

    pub fn stack_pointer_offset(&self) -> Query<F> {
        // unreachable!("not implemented yet")
        Query::zero()
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
        self.base.build(self.cs);
        while let Some(state_lookup) = self.rwtable_lookups.pop() {
            self.base.add_lookup(
                "rwtable_lookup(q_enable,rw_counter,is_write,tag,id,address,value,value_prev)",
                state_lookup,
                lookup.lookup_rwtable(),
            );
        }
    }
}
