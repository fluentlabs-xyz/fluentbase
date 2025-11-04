use fluentbase_sdk::{
    compile_wasm_to_rwasm, Bytes, RwasmCompilationResult, UPDATE_GENESIS_AUTH,
    UPDATE_GENESIS_PREFIX_V1, UPDATE_GENESIS_PREFIX_V2,
};
use revm::{
    bytecode::Bytecode,
    context::{ContextError, ContextTr, JournalTr},
    handler::{FrameResult, FrameTr, ItemOrResult},
    interpreter::{CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult},
    Database,
};
use std::boxed::Box;

macro_rules! upgrade_panic {
    ($inputs:expr, $message:literal) => {{}
    return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from($message),
            gas: Gas::new(0),
        },
        memory_offset: $inputs.return_memory_offset.clone(),
    })));};
}

pub(crate) fn upgrade_runtime_hook_v1<
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
    debug_assert!(bytecode.starts_with(&UPDATE_GENESIS_PREFIX_V1));
    let Ok(bytecode) = Bytecode::new_raw_checked(bytecode.slice(UPDATE_GENESIS_PREFIX_V1.len()..))
    else {
        upgrade_panic!(inputs, "malformed bytecode");
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

pub(crate) fn upgrade_runtime_hook_v2<
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
    debug_assert!(bytecode.starts_with(&UPDATE_GENESIS_PREFIX_V2));
    let wasm_bytecode = bytecode.slice(UPDATE_GENESIS_PREFIX_V2.len()..);
    let Ok(RwasmCompilationResult { rwasm_module, .. }) = compile_wasm_to_rwasm(&wasm_bytecode)
    else {
        upgrade_panic!(inputs, "malformed wasm bytecode");
    };
    let bytecode = Bytecode::new_rwasm(rwasm_module.serialize().into());
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
