use alloy_primitives::{address, hex, Address};
use fluentbase_sdk::{compile_wasm_to_rwasm, Bytes, RwasmCompilationResult};
use revm::{
    bytecode::Bytecode,
    context::{ContextError, ContextTr, JournalTr},
    handler::{FrameResult, FrameTr, ItemOrResult},
    interpreter::{CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult},
    Database,
};
use std::{boxed::Box, vec::Vec};

/// Authority address that is allowed to update the code of arbitrary accounts.
///
/// This is the "admin" for genesis/state upgrade operations and should be
/// treated as highly privileged.
///
/// Note: Only for Fluent Testnet, will be removed
pub(crate) const UPDATE_GENESIS_AUTH: Address =
    address!("0xa7bf6a9168fe8a111307b7c94b8883fe02b30934");

/// Transaction calldata prefix for **genesis update** (version 2).
///
/// Versioning allows introducing new update semantics without ambiguity.
///
/// Note: Only for Fluent Testnet, will be removed
pub(crate) const UPDATE_GENESIS_PREFIX_V2: [u8; 4] = hex!("0x69bc6f65");

macro_rules! upgrade_panic {
    ($inputs:expr, $message:literal) => {{}
    return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from($message),
            gas: Gas::new(0),
        },
        memory_offset: $inputs.return_memory_offset.clone(),
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    })));};
}

#[allow(clippy::type_complexity)]
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
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    })))
}
