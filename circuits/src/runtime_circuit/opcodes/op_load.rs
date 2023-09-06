use crate::{
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpLoadGadget<F> {
    _marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLoadGadget<F> {
    const NAME: &'static str = "WASM_LOAD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOAD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        Self {
            _marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let address = trace.curr_nth_stack_value(0)?;
        let res = trace.next_nth_stack_value(0)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_i32_load() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[800]
            I32Store[0]

            I32Const[0]
            I32Load[0]
            Drop
        });
    }

    #[test]
    fn test_i32_load8u() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[15]
            I32Store8[0]

            I32Const[0]
            I32Load8U[0]
            Drop
        });
    }

    #[test]
    fn test_i32_load8s_1() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[13]
            I32Store8[0]

            I32Const[0]
            I32Load8S[0]
            Drop
        });
    }

    #[test]
    fn test_i32_load8s_2() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[-13]
            I32Store8[0]

            I32Const[0]
            I32Load8S[0]
            Drop
        });
    }

    #[test]
    fn test_i32_load16u() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[801]
            I32Store[0]

            I32Const[0]
            I32Load16U[0]
            Drop
        });
    }

    #[test]
    fn test_i32_load16s() {
        test_ok(instruction_set! {
            I32Const[0]
            I32Const[-801]
            I32Store[0]

            I32Const[0]
            I32Load16S[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[802]
            I64Store[0]

            I64Const[0]
            I64Load[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load8u() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[803]
            I64Store8[0]

            I64Const[0]
            I64Load8U[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load8s() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[804]
            I64Store8[0]

            I64Const[0]
            I64Load8S[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load16u() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[805]
            I64Store[0]

            I64Const[0]
            I64Load16U[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load16s() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[806]
            I64Store[0]

            I64Const[0]
            I64Load16S[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load32u() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[807]
            I64Store[0]

            I64Const[0]
            I64Load32U[0]
            Drop
        });
    }

    #[test]
    fn test_i64_load32s() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[808]
            I64Store[0]

            I64Const[0]
            I64Load32S[0]
            Drop
        });
    }

    #[test]
    fn test_f32_load() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[20]
            F32Div
            F32Store[0]

            I32Const[0]
            F32Load[0]
            Drop
        });
    }

    #[test]
    fn test_f64_load() {
        test_ok(instruction_set! {
            I64Const[0]
            I64Const[810]
            I64Store[0]

            I64Const[0]
            F64Load[0]
            Drop
        });
    }
}
