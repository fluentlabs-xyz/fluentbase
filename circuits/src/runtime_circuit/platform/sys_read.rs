use crate::{
    constraint_builder::AdviceColumn,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

const MAX_INPUT_DEGREE: usize = 10;

#[derive(Clone)]
pub struct SysReadGadget<F: Field> {
    target: AdviceColumn,
    offset: AdviceColumn,
    length: AdviceColumn,
    result: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for SysReadGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(_sys_read)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::IMPORT_SYS_READ);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let target = cb.query_cell();
        let offset = cb.query_cell();
        let length = cb.query_cell();
        let result = cb.query_cell();

        // let degree = cb.query_fixed();
        // let degree_inv = (0..MAX_INPUT_DEGREE - 1)
        //     .map(|d| cb.is_zero(degree.current() - d.expr()))
        //     .collect();

        // make sure length is pow2
        // debug_assert_eq!(MAX_INPUT_DEGREE, 10);
        // cb.fixed_lookup(
        //     FixedTableTag::Pow2UpTo10,
        //     [degree.current(), length.current(), Query::zero()],
        // );

        Self {
            target,
            offset,
            length,
            result,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        row_offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let length = trace.curr_nth_stack_value(0)?;
        let offset = trace.curr_nth_stack_value(1)?;
        let target = trace.curr_nth_stack_value(2)?;
        self.length.assign(region, row_offset, length.as_u64());
        self.offset.assign(region, row_offset, offset.as_u64());
        self.target.assign(region, row_offset, target.as_u64());
        let result = trace.next_nth_stack_value(0)?;
        self.result.assign(region, row_offset, result.as_u64());
        Ok(())
    }
}
