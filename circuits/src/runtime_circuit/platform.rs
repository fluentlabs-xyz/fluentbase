pub(crate) mod sys_halt;
pub(crate) mod sys_read;
pub(crate) mod sys_write;

pub use crate::trace_step::{GadgetError, TraceStep};
use crate::{runtime_circuit::constraint_builder::OpConstraintBuilder, util::Field};
use halo2_proofs::circuit::Region;

pub trait PlatformGadget<F: Field, const SYS_CODE: u32> {
    fn sys_code() -> u32 {
        SYS_CODE
    }

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self;

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError>;
}
