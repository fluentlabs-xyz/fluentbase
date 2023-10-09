use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct RwasmTransactGadget<F: Field> {
    output_len: AdviceColumn,
    output_offset: AdviceColumn,
    input_len: AdviceColumn,
    input_offset: AdviceColumn,
    code_len: AdviceColumn,
    code_offset: AdviceColumn,
    exit_code: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for RwasmTransactGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(env::rwasm_transact)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::RWASM_TRANSACT);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let [output_len, output_offset, input_len, input_offset, code_len, code_offset, exit_code] =
            cb.query_cells();
        // pop input params
        // cb.stack_pop(output_len.current());
        // cb.stack_pop(output_offset.current());
        // cb.stack_pop(input_len.current());
        // cb.stack_pop(input_offset.current());
        // cb.stack_pop(code_len.current());
        // cb.stack_pop(code_offset.current());
        // push exit code
        // cb.stack_push(exit_code.current());
        // lookup call depth (make sure its increased by 1)
        // cb.context_lookup(
        //     RwTableContextTag::CallDepth,
        //     1.expr(),
        //     cb.call_id(),
        //     cb.call_id() - 1,
        // );
        Self {
            output_len,
            output_offset,
            input_len,
            input_offset,
            code_len,
            code_offset,
            exit_code,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let exit_code = trace.curr_nth_stack_value(0)?;
        self.exit_code.assign(region, offset, exit_code.as_u64());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_runtime::SysFuncIdx;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_transact() {
        let bytecode: Vec<u8> = instruction_set! {
            I32Const(100)
            I32Const(20)
            I32Add
            I32Const(3)
            I32Add
            Drop
        }
        .into();
        test_ok(instruction_set! {
            .add_memory(0, bytecode.as_slice())
            I32Const(0) // code offset
            I32Const(bytecode.len() as u32) // code len
            I32Const(0) // input offset
            I32Const(0) // input len
            I32Const(0) // output offset
            I32Const(0) // output len
            Call(SysFuncIdx::RWASM_TRANSACT)
            Drop
        });
    }
}
