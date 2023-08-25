mod op_const;
mod op_drop;
mod op_local;

use crate::{runtime_circuit::constraint_builder::OpConstraintBuilder, util::Field};
use halo2_proofs::{circuit::Region, plonk::Error};

pub(crate) trait ExecutionGadget<F: Field> {
    const NAME: &'static str;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self;

    fn assign_exec_step(&self, region: &mut Region<'_, F>, offset: usize) -> Result<(), Error>;
}
