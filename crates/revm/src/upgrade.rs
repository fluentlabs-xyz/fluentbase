use fluentbase_sdk::UPDATE_GENESIS_PREFIX;
use fluentbase_sdk::{Bytes, UPDATE_GENESIS_AUTH};
use revm::bytecode::Bytecode;
use revm::context::{ContextError, ContextTr, JournalTr};
use revm::handler::{FrameResult, FrameTr, ItemOrResult};
use revm::interpreter::{CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult};
use revm::Database;

pub(crate) fn upgrade_runtime_hook<
    'a,
    CTX: ContextTr,
    FRAME: FrameTr<FrameResult = FrameResult>,
>(
    ctx: &mut CTX,
    inputs: &mut Box<CallInputs>,
) -> Result<
    ItemOrResult<&'a mut FRAME, <FRAME as FrameTr>::FrameResult>,
    ContextError<<<CTX as ContextTr>::Db as Database>::Error>,
> {
    debug_assert_eq!(inputs.caller, UPDATE_GENESIS_AUTH);
    let bytecode = inputs.input.bytes(ctx);
    debug_assert!(bytecode.starts_with(&UPDATE_GENESIS_PREFIX));
    let Ok(bytecode) = Bytecode::new_raw_checked(bytecode.slice(UPDATE_GENESIS_PREFIX.len()..))
    else {
        return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
            result: InterpreterResult {
                result: InstructionResult::Revert,
                output: Bytes::from("malformed bytecode"),
                gas: Gas::new(0),
            },
            memory_offset: inputs.return_memory_offset.clone(),
        })));
    };
    ctx.journal_mut().set_code(inputs.target_address, bytecode);
    Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Default::default(),
            gas: Gas::new(0),
        },
        memory_offset: inputs.return_memory_offset.clone(),
    })))
}
