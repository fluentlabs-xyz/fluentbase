use crate::{
    constraint_builder::{
        dynamic_cell_manager::{CellType, DynamicCellManager},
        AdviceColumn,
        AdviceColumnPhase2,
        BinaryQuery,
        ConstraintBuilder,
        FixedColumn,
        Query,
        SelectorColumn,
        ToExpr,
    },
    fixed_table::FixedTableTag,
    gadgets::{is_zero::IsZeroConfig, lt::LtGadget},
    lookup_table::{
        BitwiseCheckLookup,
        CopyLookup,
        FixedLookup,
        LookupTable,
        PublicInputLookup,
        RangeCheckLookup,
        ResponsibleOpcodeLookup,
        RwLookup,
        RwasmLookup,
    },
    runtime_circuit::execution_state::ExecutionState,
    rw_builder::{
        copy_row::CopyTableTag,
        rw_row::{RwTableContextTag, RwTableTag},
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::{circuit::Region, plonk::ConstraintSystem};

#[derive(Clone)]
pub struct StateTransition<F: Field> {
    pub(crate) stack_pointer: AdviceColumn,
    pub(crate) stack_pointer_offset: Query<F>,
    pub(crate) rw_counter: AdviceColumn,
    pub(crate) rw_counter_offset: Query<F>,
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
        self.rw_counter.assign(region, offset, rw_counter);
    }

    pub fn stack_pointer(&self) -> Query<F> {
        self.stack_pointer.current() + self.stack_pointer_offset.clone()
    }

    pub fn rw_counter(&self) -> Query<F> {
        self.rw_counter.current() + self.rw_counter_offset.clone()
    }
}

pub struct OpConstraintBuilder<'cs, 'st, 'dcm, F: Field> {
    q_enable: SelectorColumn,
    pub(crate) base: ConstraintBuilder<F>,
    cs: &'cs mut ConstraintSystem<F>,
    dcm: &'dcm mut DynamicCellManager<F>,
    // rwasm table fields
    pc: AdviceColumn,
    opcode: AdviceColumn,
    value: AdviceColumn,
    // rw fields
    state_transition: &'st mut StateTransition<F>,
    op_lookups: Vec<LookupTable<F>>,
}

#[allow(unused_variables)]
impl<'cs, 'st, 'dcm, F: Field> OpConstraintBuilder<'cs, 'st, 'dcm, F> {
    pub fn new(
        cs: &'cs mut ConstraintSystem<F>,
        dcm: &'dcm mut DynamicCellManager<F>,
        q_enable: SelectorColumn,
        state_transition: &'st mut StateTransition<F>,
    ) -> Self {
        let pc = AdviceColumn(cs.advice_column());
        let opcode = AdviceColumn(cs.advice_column());
        let value = AdviceColumn(cs.advice_column());
        Self {
            q_enable,
            base: ConstraintBuilder::new(q_enable),
            cs,
            dcm,
            pc,
            opcode,
            value,
            state_transition,
            op_lookups: vec![],
        }
    }

    pub fn rwasm_table(&self) -> [AdviceColumn; 3] {
        [self.pc.clone(), self.opcode.clone(), self.value.clone()]
    }

    pub fn query_rwasm_opcode(&self) -> Query<F> {
        self.opcode.current()
    }

    pub fn query_rwasm_value(&self) -> Query<F> {
        self.value.current()
    }

    pub fn require_exactly_one_selector<const N: usize>(&mut self, selectors: [Query<F>; N]) {
        let sum: Query<F> = selectors.iter().fold(0.expr(), |r, q| r + q.clone());
        self.require_zero("exactly one selector must be enabled", sum - 1.expr());
    }

    pub fn if_rwasm_opcode(
        &mut self,
        selector: Query<F>,
        instr: Instruction,
        configure: impl FnOnce(&mut Self),
    ) {
        self.condition(selector, |cb| {
            cb.require_opcode(instr);
            configure(cb);
        });
    }

    pub fn assert_unreachable(&mut self, name: &'static str) {
        self.base.assert_unreachable(name)
    }

    pub fn query_cell(&mut self) -> AdviceColumn {
        self.query_cells::<1>()[0]
    }

    pub fn query_cells<const N: usize>(&mut self) -> [AdviceColumn; N] {
        self.dcm
            .query_cells(self.cs, &CellType::Advice)
            .map(|v| v.try_into().unwrap())
            .try_into()
            .unwrap()
    }

    pub fn query_selector(&mut self) -> SelectorColumn {
        self.query_selectors::<1>()[0]
    }

    pub fn query_selectors<const N: usize>(&mut self) -> [SelectorColumn; N] {
        self.dcm
            .query_cells(self.cs, &CellType::Selector)
            .map(|v| v.try_into().unwrap())
            .try_into()
            .unwrap()
    }

    pub fn query_fixed(&mut self) -> FixedColumn {
        self.base.fixed_column(self.cs)
    }

    pub fn is_zero_gadget(&mut self, value: Query<F>) -> IsZeroConfig<F> {
        IsZeroConfig::configure(self.cs, &mut self.base, value)
    }

    pub fn lt_gadget<const N_BYTES: usize>(
        &mut self,
        lhs: Query<F>,
        rhs: Query<F>,
    ) -> LtGadget<F, N_BYTES> {
        LtGadget::configure(self.cs, &mut self.base, lhs, rhs)
    }

    pub fn query_fixed_n<const N: usize>(&mut self) -> [FixedColumn; N] {
        [0; N].map(|_| self.base.fixed_column(self.cs))
    }

    pub fn query_cell_phase2(&mut self) -> AdviceColumnPhase2 {
        self.base.advice_column_phase2(self.cs)
    }

    pub fn next_pc_delta(&mut self, delta: Query<F>) {
        self.context_lookup(
            RwTableContextTag::ProgramCounter,
            1.expr(),
            self.pc.current() + delta,
            self.pc.current(),
        );
    }

    pub fn next_pc_jump(&mut self, value: Query<F>) {
        self.context_lookup(
            RwTableContextTag::ProgramCounter,
            1.expr(),
            value,
            self.pc.current(),
        );
    }

    pub fn next_sp_delta(&mut self, delta: Query<F>) {
        self.context_lookup(
            RwTableContextTag::StackPointer,
            1.expr(),
            self.state_transition.stack_pointer.current() + delta,
            self.state_transition.stack_pointer.current(),
        );
    }

    pub fn stack_push(&mut self, value: Query<F>) {
        self.state_transition.stack_pointer_offset =
            self.state_transition.stack_pointer_offset.clone() - self.base.resolve_condition().0;
        self.stack_lookup(Query::one(), self.state_transition.stack_pointer(), value);
    }

    pub fn stack_pop(&mut self, value: Query<F>) {
        self.stack_lookup(Query::zero(), self.state_transition.stack_pointer(), value);
        self.state_transition.stack_pointer_offset =
            self.state_transition.stack_pointer_offset.clone() + self.base.resolve_condition().0;
    }

    pub fn stack_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rw_lookup(is_write, RwTableTag::Stack.expr(), address, value, None);
    }

    pub fn context_lookup(
        &mut self,
        tag: RwTableContextTag,
        is_write: Query<F>,
        value: Query<F>,
        value_prev: Query<F>,
    ) {
        self.rw_lookup(
            is_write,
            RwTableTag::Context.expr(),
            tag.expr(),
            value,
            Some(value_prev),
        );
    }

    pub fn global_get(&mut self, index: Query<F>, value: Query<F>) {
        self.global_lookup(Query::zero(), index, value);
    }

    pub fn global_set(&mut self, index: Query<F>, value: Query<F>) {
        self.global_lookup(Query::one(), index, value);
    }

    pub fn mem_write(&mut self, address: Query<F>, value: Query<F>) {
        self.memory_lookup(Query::one(), address, value);
    }

    pub fn mem_read(&mut self, address: Query<F>, value: Query<F>) {
        self.memory_lookup(Query::zero(), address, value);
    }

    pub fn memory_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rw_lookup(is_write, RwTableTag::Memory.expr(), address, value, None);
    }

    pub fn global_lookup(&mut self, is_write: Query<F>, address: Query<F>, value: Query<F>) {
        self.rw_lookup(is_write, RwTableTag::Global.expr(), address, value, None);
    }

    pub fn execution_state_lookup(&mut self, execution_state: ExecutionState, opcode: Query<F>) {
        self.op_lookups.push(LookupTable::ResponsibleOpcode(
            self.base.apply_lookup_condition([
                Query::Constant(F::from(execution_state.to_u64())),
                opcode,
            ]),
        ));
    }

    pub fn table_size(&mut self, table_idx: Query<F>, value: Query<F>) {
        self.table_size_lookup(0.expr(), table_idx * 1024, value);
    }
    pub fn table_fill(
        &mut self,
        table_index: Query<F>,
        start: Query<F>,
        range: Query<F>,
        value: Query<F>,
    ) {
        // unreachable!("not implemented yet")
    }
    pub fn table_grow(
        &mut self,
        table_idx: Query<F>,
        init: Query<F>,
        grow: Query<F>,
        res: Query<F>,
    ) {
        self.table_size_lookup(0.expr(), table_idx.clone(), res.clone());
        self.table_size_lookup(1.expr(), table_idx.clone(), res.clone() + grow.clone());
        self.table_fill(table_idx, res, grow, init);
    }
    pub fn table_get(&mut self, table_idx: Query<F>, elem_idx: Query<F>, value: Query<F>) {
        self.table_elem_lookup(0.expr(), table_idx, elem_idx, value);
    }
    pub fn table_set(&mut self, table_idx: Query<F>, elem_idx: Query<F>, value: Query<F>) {
        self.table_elem_lookup(1.expr(), table_idx, elem_idx, value);
    }
    pub fn table_copy(
        &mut self,
        table_index: Query<F>,
        table_index2: Query<F>,
        start: Query<F>,
        range: Query<F>,
    ) {
        // unreachable!("not implemented yet")
    }
    pub fn table_init(
        &mut self,
        table_index: Query<F>,
        table_index2: Query<F>,
        start: Query<F>,
        range: Query<F>,
    ) {
        // unreachable!("not implemented yet")
    }

    pub fn table_elem_lookup(
        &mut self,
        is_write: Query<F>,
        table_idx: Query<F>,
        elem_idx: Query<F>,
        value: Query<F>,
    ) {
        // address = 1 + a + b*x, where x is 1024. Adding one used to reserve element to store size.
        self.rw_lookup(
            is_write,
            RwTableTag::Table.expr(),
            table_idx * 1024 + elem_idx + 1.expr(),
            value,
            None,
        );
    }

    pub fn table_size_lookup(&mut self, is_write: Query<F>, table_idx: Query<F>, value: Query<F>) {
        // address = b*x, where x is 1024. So this is reserved element to store size.
        self.rw_lookup(
            is_write,
            RwTableTag::Table.expr(),
            table_idx * 1024,
            value,
            None,
        );
    }

    pub fn range_check_1024(&mut self, value: Query<F>) {
        // unreachable!("not implemented yet")
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

    pub fn fixed_lookup(&mut self, tag: FixedTableTag, table: [Query<F>; 3]) {
        self.op_lookups
            .push(LookupTable::Fixed(self.base.apply_lookup_condition([
                tag.expr(),
                table[0].clone(),
                table[1].clone(),
                table[2].clone(),
            ])))
    }

    pub fn copy_lookup(
        &mut self,
        tag: CopyTableTag,
        from_address: Query<F>,
        to_address: Query<F>,
        length: Query<F>,
    ) {
        self.op_lookups
            .push(LookupTable::Copy(self.base.apply_lookup_condition([
                Query::one(),
                Query::one(),
                tag.expr(),
                from_address,
                to_address,
                length,
                self.state_transition.rw_counter(),
            ])))
    }

    pub fn rw_lookup(
        &mut self,
        is_write: Query<F>,
        tag: Query<F>,
        address: Query<F>,
        value: Query<F>,
        value_prev: Option<Query<F>>,
    ) {
        if let Some(value_prev) = value_prev {
            self.op_lookups
                .push(LookupTable::RwPrev(self.base.apply_lookup_condition([
                    Query::one(),
                    self.state_transition.rw_counter(),
                    is_write,
                    tag,
                    Query::zero(),
                    address,
                    value,
                    value_prev,
                ])));
        } else {
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
        }
        self.state_transition.rw_counter_offset =
            self.state_transition.rw_counter_offset.clone() + self.base.resolve_condition().0;
    }

    pub fn range_check7(&mut self, val: Query<F>) {
        self.op_lookups.push(LookupTable::RangeCheck7([val]));
    }
    pub fn range_check8(&mut self, val: Query<F>) {
        self.op_lookups.push(LookupTable::RangeCheck8([val]));
    }
    pub fn bitwise_and(&mut self, lhs: Query<F>, rhs: Query<F>, res: Query<F>) {
        self.op_lookups
            .push(LookupTable::BitwiseAnd([lhs, rhs, res]));
    }
    pub fn bitwise_or(&mut self, lhs: Query<F>, rhs: Query<F>, res: Query<F>) {
        self.op_lookups
            .push(LookupTable::BitwiseOr([lhs, rhs, res]));
    }
    pub fn bitwise_xor(&mut self, lhs: Query<F>, rhs: Query<F>, res: Query<F>) {
        self.op_lookups
            .push(LookupTable::BitwiseXor([lhs, rhs, res]));
    }

    pub fn public_input_lookup(&mut self, index: Query<F>, value: Query<F>) {
        self.op_lookups.push(LookupTable::PublicInput(
            self.base
                .apply_lookup_condition([Query::one(), index, value]),
        ))
    }

    pub fn exit_code_lookup(&mut self, exit_code: Query<F>) {
        self.op_lookups.push(LookupTable::ExitCode(
            self.base.apply_lookup_condition([Query::one(), exit_code]),
        ))
    }

    pub fn stack_pointer(&self) -> Query<F> {
        self.state_transition.stack_pointer()
    }

    pub fn require_equal(&mut self, name: &'static str, left: Query<F>, right: Query<F>) {
        self.base.assert_zero(name, left - right)
    }

    pub fn require_zero(&mut self, name: &'static str, expr: Query<F>) {
        self.base.assert_zero(name, expr)
    }

    pub fn require_boolean(&mut self, name: &'static str, expr: Query<F>) {
        self.base.assert_boolean(name, expr)
    }

    pub fn require_zeros(&mut self, name: &'static str, expr: Vec<Query<F>>) {
        assert!(expr.len() > 0);
        expr.iter().for_each(|v| self.require_zero(name, v.clone()));
    }

    pub fn require_opcode(&mut self, instr: Instruction) {
        self.require_equal(
            "opcode matches specific instr",
            self.opcode.current(),
            instr.code_value().expr(),
        );
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
        range_check_lookup: &impl RangeCheckLookup<F>,
        fixed_lookup: &impl FixedLookup<F>,
        public_input_lookup: &impl PublicInputLookup<F>,
        copy_lookup: &impl CopyLookup<F>,
        bitwise_check_lookup: &impl BitwiseCheckLookup<F>,
    ) {
        for state_lookup in self.op_lookups.iter() {
            match state_lookup {
                LookupTable::Rwasm(fields) => {
                    self.base.add_lookup(
                        "rwasm_lookup(offset,code,value)",
                        fields.clone(),
                        rwasm_lookup.lookup_rwasm_table(),
                    );
                }
                LookupTable::Rw(fields) => {
                    self.base.add_lookup(
                        "rw_lookup(rw_counter,is_write,tag,id,address,value)",
                        fields.clone(),
                        rw_lookup.lookup_rw_table(),
                    );
                }
                LookupTable::RwPrev(fields) => {
                    self.base.add_lookup(
                        "rw_lookup(rw_counter,is_write,tag,id,address,value,value_prev)",
                        fields.clone(),
                        rw_lookup.lookup_rw_prev_table(),
                    );
                }
                LookupTable::ResponsibleOpcode(fields) => {
                    self.base.add_lookup(
                        "responsible_opcode(execution_state,opcode,stack_len)",
                        fields.clone(),
                        responsible_opcode_lookup.lookup_responsible_opcode_table(),
                    );
                }
                LookupTable::RangeCheck7(fields) => {
                    self.base.add_lookup(
                        "range_check7",
                        fields.clone(),
                        range_check_lookup.lookup_u7_table(),
                    );
                }
                LookupTable::RangeCheck8(fields) => {
                    self.base.add_lookup(
                        "range_check_u8",
                        fields.clone(),
                        range_check_lookup.lookup_u8_table(),
                    );
                }
                LookupTable::RangeCheck10(fields) => {
                    self.base.add_lookup(
                        "range_check_u10",
                        fields.clone(),
                        range_check_lookup.lookup_u10_table(),
                    );
                }
                LookupTable::RangeCheck16(fields) => {
                    self.base.add_lookup(
                        "range_check_u16",
                        fields.clone(),
                        range_check_lookup.lookup_u16_table(),
                    );
                }
                LookupTable::BitwiseAnd(fields) => {
                    self.base.add_lookup(
                        "bitwise_and(lhs,rhs,res)",
                        fields.clone(),
                        bitwise_check_lookup.lookup_and(),
                    );
                }
                LookupTable::BitwiseOr(fields) => {
                    self.base.add_lookup(
                        "bitwise_or(lhs,rhs,res)",
                        fields.clone(),
                        bitwise_check_lookup.lookup_or(),
                    );
                }
                LookupTable::BitwiseXor(fields) => {
                    self.base.add_lookup(
                        "bitwise_xor(lhs,rhs,res)",
                        fields.clone(),
                        bitwise_check_lookup.lookup_xor(),
                    );
                }
                LookupTable::Fixed(fields) => {
                    self.base.add_lookup(
                        "fixed(tag,table)",
                        fields.clone(),
                        fixed_lookup.lookup_fixed_table(),
                    );
                }
                LookupTable::PublicInput(fields) => {
                    self.base.add_lookup(
                        "public_input(index,value)",
                        fields.clone(),
                        public_input_lookup.lookup_input_byte(),
                    );
                }
                LookupTable::PublicOutput(fields) => {
                    self.base.add_lookup(
                        "public_output(index,value)",
                        fields.clone(),
                        public_input_lookup.lookup_output_byte(),
                    );
                }
                LookupTable::ExitCode(fields) => {
                    self.base.add_lookup(
                        "exit_code(value)",
                        fields.clone(),
                        public_input_lookup.lookup_exit_code(),
                    );
                }
                LookupTable::Copy(fields) => {
                    self.base.add_lookup(
                        "copy(tag,from_address,to_address,length,rw_counter)",
                        fields.clone(),
                        copy_lookup.lookup_copy_table(),
                    );
                }
            }
        }
        self.op_lookups.clear();
        self.base.build(self.cs);
    }
}
