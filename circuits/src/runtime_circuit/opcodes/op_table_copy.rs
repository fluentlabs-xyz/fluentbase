use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::MAX_TABLE_SIZE,
    gadgets::lt::LtGadget,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::{copy_row::CopyTableTag, rw_row::RwTableContextTag},
    util::Field,
};
use fluentbase_runtime::ExitCode;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpTableCopyGadget<F: Field> {
    dst_eidx: AdviceColumn,
    src_eidx: AdviceColumn,
    length: AdviceColumn,
    size_dst: AdviceColumn,
    size_src: AdviceColumn,
    src_ti: AdviceColumn,
    lt_gadget_dst: LtGadget<F, 2>,
    lt_gadget_src: LtGadget<F, 2>,
    _pd: PhantomData<F>,
}

const RANGE_THRESHOLD: usize = MAX_TABLE_SIZE * 2 + 1;

impl<F: Field> ExecutionGadget<F> for OpTableCopyGadget<F> {
    const NAME: &'static str = "WASM_TABLE_COPY";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_COPY;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let dst_eidx = cb.query_cell();
        let src_eidx = cb.query_cell();
        let length = cb.query_cell();
        let size_dst = cb.query_cell();
        let size_src = cb.query_cell();
        let src_ti = cb.query_cell();

        cb.range_check_1024(dst_eidx.current());
        cb.range_check_1024(src_eidx.current());
        cb.range_check_1024(length.current());
        cb.range_check_1024(size_dst.current());
        cb.range_check_1024(size_src.current());
        cb.range_check_1024(src_ti.current());
        cb.range_check_1024(cb.query_rwasm_value());

        let last_point_dst =
            size_dst.current() - (dst_eidx.current() + length.current()) - 1.expr();
        let last_point_src =
            size_src.current() - (src_eidx.current() + length.current()) - 1.expr();
        // 1024 - 1024 - 1 + 2049 is minimum value.
        // 1 or more is valid + 2048, so less than 2049 in error.
        // less than 2049 is out of valid range, so we checking it.
        // 2049 is RANGE_THRESHOLD now.
        let threshold_dst = last_point_dst + RANGE_THRESHOLD.expr();
        let threshold_src = last_point_src + RANGE_THRESHOLD.expr();
        let lt_gadget_dst = cb.lt_gadget(threshold_dst, RANGE_THRESHOLD.expr());
        let lt_gadget_src = cb.lt_gadget(threshold_src, RANGE_THRESHOLD.expr());

        let error_case =
            || 1.expr() - (1.expr() - lt_gadget_dst.expr()) * (1.expr() - lt_gadget_src.expr());

        // So in case of exit code causing error, pc delta must be different.
        // To solve this problem `configure_state_transition` is defined with disabled constraint.
        cb.condition(1.expr() - error_case(), |cb| {
            cb.next_pc_delta((9 * 2).expr());
        });

        cb.condition(error_case(), |cb| {
            // make sure proper exit code is set
            cb.exit_code_lookup((ExitCode::TableOutOfBounds as i32 as u64).expr());

            // If exit code causing error than nothing is written, but we need to shift.
            cb.draft_shift(1, 0);
        });

        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            0.expr(),
            size_dst.current(),
            None,
        );

        cb.context_lookup(
            RwTableContextTag::TableSize(src_ti.current()),
            0.expr(),
            size_src.current(),
            None,
        );

        cb.stack_pop(length.current());
        cb.stack_pop(src_eidx.current());
        cb.stack_pop(dst_eidx.current());

        cb.condition(1.expr() - error_case(), |cb| {
            cb.copy_lookup(
                CopyTableTag::CopyTable,
                src_ti.current() * 1024.expr() + src_eidx.current(),
                cb.query_rwasm_value() * 1024.expr() + dst_eidx.current(),
                length.current(),
            );
        });

        Self {
            dst_eidx,
            src_eidx,
            length,
            size_dst,
            size_src,
            src_ti,
            lt_gadget_dst,
            lt_gadget_src,
            _pd: Default::default(),
        }
    }

    fn configure_state_transition(_cb: &mut OpConstraintBuilder<F>) {
        //cb.next_pc_delta((9*2).expr());
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let dst_ti = trace.instr().aux_value().unwrap_or_default().as_u32();
        let src_ti = trace.curr().next_table_idx.unwrap().to_u32();
        println!("DEBUG DST_TI {}, SRC_TI {}", dst_ti, src_ti);

        let length = trace.curr_nth_stack_value(0)?;
        let src_eidx = trace.curr_nth_stack_value(1)?;
        let dst_eidx = trace.curr_nth_stack_value(2)?;
        let size_src = trace.read_table_size(src_ti);
        let size_dst = trace.read_table_size(dst_ti);

        self.dst_eidx
            .assign(region, offset, F::from(dst_eidx.to_bits()));
        self.src_eidx
            .assign(region, offset, F::from(src_eidx.to_bits()));
        self.length
            .assign(region, offset, F::from(length.to_bits()));
        self.size_src
            .assign(region, offset, F::from(size_src as u64));
        self.size_dst
            .assign(region, offset, F::from(size_dst as u64));
        self.src_ti.assign(region, offset, F::from(src_ti as u64));

        self.lt_gadget_dst.assign(
            region,
            offset,
            F::from(
                (size_dst as i64 - (dst_eidx.to_bits() + length.to_bits()) as i64 - 1
                    + RANGE_THRESHOLD as i64) as u64,
            ),
            F::from(RANGE_THRESHOLD as u64),
        );

        self.lt_gadget_src.assign(
            region,
            offset,
            F::from(
                (size_src as i64 - (src_eidx.to_bits() + length.to_bits()) as i64 - 1
                    + RANGE_THRESHOLD as i64) as u64,
            ),
            F::from(RANGE_THRESHOLD as u64),
        );
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_copy_simple() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(2)
            Drop
            I32Const(1)
            I32Const(2)
            I32Const(3)
            TableCopy(0)
            TableGet(2)
        });
    }

    #[test]
    fn table_copy_set_first() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(2)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(1)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(0)

            I32Const(2)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(1)
            I32Const(2)
            I32Const(3)
            TableCopy(0)
            TableGet(2)
        });
    }

    #[test]
    fn table_copy_set_first_out_of_bound() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(2)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(1)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(0)

            I32Const(2)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(1)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(2)
        });
    }

    #[test]
    fn table_copy_set_second() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(1)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(1)

            I32Const(1)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(1)

            I32Const(2)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(1)

            I32Const(1)
            I32Const(2)
            I32Const(3)
            TableCopy(0)
            TableGet(1)
        });
    }

    #[test]
    fn table_copy_set_second_with_set_zero() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(1)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(1)

            I32Const(1)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(1)

            I32Const(2)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(1)

            I32Const(0)
            TableGet(1)
            Drop

            I32Const(1)
            TableGet(1)
            Drop

            I32Const(2)
            TableGet(1)
            Drop

            I32Const(1)
            I32Const(2)
            I32Const(3)
            TableCopy(0)
            TableGet(1)
        });
    }

    #[test]
    fn table_copy_set_second_out_of_bounds() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(1)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(1)

            I32Const(1)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(1)

            I32Const(2)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(1)

            I32Const(1)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(1)
        });
    }

    #[test]
    fn table_copy_overlap_zeros() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop

            I32Const(0)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(1)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(2)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(3)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(4)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(5)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(0)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(0)
        });
    }

    #[test]
    fn table_copy_overlap_same_idx() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(1)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(2)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(3)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(4)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(5)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(0)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(0)
        });
    }

    #[test]
    fn table_copy_overlap_pat() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop

            I32Const(0)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(1)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(2)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(3)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(4)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(5)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(0)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(0)
        });
    }

    #[test]
    fn table_copy_overlap_seq() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop

            I32Const(0)
            I32Const(0) // TODO: RefFunc(0)
            TableSet(0)

            I32Const(1)
            I32Const(1) // TODO: RefFunc(1)
            TableSet(0)

            I32Const(2)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(0)

            I32Const(3)
            I32Const(3) // TODO: RefFunc(3)
            TableSet(0)

            I32Const(4)
            I32Const(4) // TODO: RefFunc(4)
            TableSet(0)

            I32Const(5)
            I32Const(5) // TODO: RefFunc(5)
            TableSet(0)

            I32Const(0)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(0)
        });
    }

    #[test]
    fn table_copy_src_out_of_bounds() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(1)
            Drop
            I32Const(1)
            I32Const(2)
            I32Const(4)
            TableCopy(0)
            TableGet(1)
        });
    }

    #[test]
    fn table_copy_dst_out_of_bounds() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(6)
            TableGrow(1)
            Drop
            I32Const(2)
            I32Const(1)
            I32Const(4)
            TableCopy(0)
            TableGet(1)
        });
    }
}
