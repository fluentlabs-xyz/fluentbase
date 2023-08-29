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
}

use Query as Q;

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

    pub fn stack_push(&mut self, value: Query<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn stack_pop(&mut self, value: Query<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn stack_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        // unreachable!("not implemented yet")
    }

    pub fn global_get(&mut self, index: Query<F>, value: Query<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn global_set(&mut self, index: Query<F>, value: Query<F>) {
        // unreachable!("not implemented yet")
    }

    pub fn execution_state_lookup(&mut self, execution_state: ExecutionState) {
        // unreachable!("not implemented yet")
    }

    pub fn table_size(&mut self, table_index: Q<F>, value: Q<F>) {
        // unreachable!("not implemented yet")
    }
    pub fn table_fill(&mut self, table_index: Q<F>, start: Q<F>, range: Q<F>, value: Q<F>) {
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

    pub fn build(&mut self) {
        self.base.build(self.cs);
    }
}
