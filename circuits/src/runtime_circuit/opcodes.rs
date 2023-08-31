pub(crate) mod op_const;
pub(crate) mod op_drop;
pub(crate) mod op_local;
pub(crate) mod table_ops;

pub use crate::{
    runtime_circuit::{constraint_builder::OpConstraintBuilder, execution_state::ExecutionState},
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use halo2_proofs::circuit::Region;

pub trait ExecutionGadget<F: Field> {
    const NAME: &'static str;

    const EXECUTION_STATE: ExecutionState;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self;

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError>;
}

#[macro_export]
macro_rules! bail_illegal_opcode {
    ($trace:expr) => {
        unreachable!("illegal opcode place {:?}", $trace)
    };
}
