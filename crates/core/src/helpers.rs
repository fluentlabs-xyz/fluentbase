use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use core::mem::take;
use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCreateMethodInput},
    SovereignAPI,
};
use fluentbase_types::{
    create_sovereign_import_linker,
    Address,
    ExitCode,
    NativeAPI,
    SharedAPI,
    SysFuncIdx::STATE,
    STATE_DEPLOY,
    STATE_MAIN,
};
use revm_interpreter::{
    opcode::make_instruction_table,
    CallInputs,
    CallOutcome,
    Contract,
    CreateInputs,
    CreateOutcome,
    Gas,
    InstructionResult,
    Interpreter,
    InterpreterAction,
    InterpreterResult,
    SharedMemory,
};
use revm_primitives::{CancunSpec, CreateScheme};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
};

#[macro_export]
macro_rules! decode_method_input {
    ($core_input: ident, $method_input: ident) => {{
        let mut buffer = BufferDecoder::new(&mut $core_input.method_data);
        let mut method_input = $method_input::default();
        $method_input::decode_body(&mut buffer, 0, &mut method_input);
        method_input
    }};
}

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_sovereign_import_linker()),
        wrap_import_functions: true,
    });
    let rwasm_module = RwasmModule::compile_with_config(wasm_binary, &config)
        .map_err(|_| ExitCode::CompilationError)?;
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}

#[macro_export]
macro_rules! result_value {
    ($result:expr) => {
        match $result {
            Ok(v) => v,
            Err(v) => v,
        }
    };
}

#[cfg(feature = "e2e")]
#[macro_export]
macro_rules! debug_log {
    ($sdk:expr, $msg:tt) => {{
        $sdk.native_sdk().debug_log(&$msg);
    }};
    ($sdk:expr, $($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!($sdk, msg);
    }};
}
#[cfg(not(feature = "e2e"))]
#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{}};
    ($($arg:tt)*) => {{}};
}

pub fn evm_error_from_exit_code(exit_code: ExitCode) -> InstructionResult {
    match exit_code {
        ExitCode::Ok => InstructionResult::Stop,
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::CallDepthOverflow => InstructionResult::CallTooDeep,
        ExitCode::InsufficientBalance => InstructionResult::OutOfFunds,
        ExitCode::OutOfGas => InstructionResult::OutOfGas,
        ExitCode::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        ExitCode::WriteProtection => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::InvalidEfOpcode => InstructionResult::InvalidFEOpcode,
        ExitCode::InvalidJump => InstructionResult::InvalidJump,
        // ExitCode::NotActivated => InstructionResult::NotActivated,
        ExitCode::StackUnderflow => InstructionResult::StackUnderflow,
        ExitCode::StackOverflow => InstructionResult::StackOverflow,
        ExitCode::OutputOverflow => InstructionResult::OutOfOffset,
        ExitCode::CreateCollision => InstructionResult::CreateCollision,
        ExitCode::OverflowPayment => InstructionResult::OverflowPayment,
        ExitCode::PrecompileError => InstructionResult::PrecompileError,
        ExitCode::NonceOverflow => InstructionResult::NonceOverflow,
        ExitCode::ContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        ExitCode::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        ExitCode::FatalExternalError => InstructionResult::FatalExternalError,
        // ExitCode::ReturnContract => InstructionResult::ReturnContract,
        // ExitCode::ReturnContractInNotInitEOF => InstructionResult::ReturnContractInNotInitEOF,
        // ExitCode::EOFOpcodeDisabledInLegacy => InstructionResult::EOFOpcodeDisabledInLegacy,
        // ExitCode::EOFFunctionStackOverflow => InstructionResult::EOFFunctionStackOverflow,
        // TODO(dmitry123): "what's proper unknown error code mapping?"
        _ => InstructionResult::OutOfGas,
    }
}

pub fn exit_code_from_evm_error(evm_error: InstructionResult) -> ExitCode {
    match evm_error {
        InstructionResult::Continue
        | InstructionResult::Stop
        | InstructionResult::Return
        | InstructionResult::SelfDestruct
        | InstructionResult::CallOrCreate => ExitCode::Ok,
        InstructionResult::Revert => ExitCode::Panic,
        InstructionResult::CallTooDeep => ExitCode::CallDepthOverflow,
        InstructionResult::OutOfFunds => ExitCode::InsufficientBalance,
        InstructionResult::OutOfGas
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG => ExitCode::OutOfGas,
        InstructionResult::OpcodeNotFound => ExitCode::OpcodeNotFound,
        InstructionResult::CallNotAllowedInsideStatic
        | InstructionResult::StateChangeDuringStaticCall => ExitCode::WriteProtection,
        InstructionResult::InvalidFEOpcode => ExitCode::InvalidEfOpcode,
        InstructionResult::InvalidJump => ExitCode::InvalidJump,
        // InstructionResult::NotActivated => ExitCode::NotActivated,
        InstructionResult::StackUnderflow => ExitCode::StackUnderflow,
        InstructionResult::StackOverflow => ExitCode::StackOverflow,
        InstructionResult::OutOfOffset => ExitCode::OutputOverflow,
        InstructionResult::CreateCollision => ExitCode::CreateCollision,
        InstructionResult::OverflowPayment => ExitCode::OverflowPayment,
        InstructionResult::PrecompileError => ExitCode::PrecompileError,
        InstructionResult::NonceOverflow => ExitCode::NonceOverflow,
        InstructionResult::CreateContractSizeLimit | InstructionResult::CreateInitCodeSizeLimit => {
            ExitCode::ContractSizeLimit
        }
        InstructionResult::CreateContractStartingWithEF => ExitCode::CreateContractStartingWithEF,
        InstructionResult::FatalExternalError => ExitCode::FatalExternalError,
        // InstructionResult::ReturnContract => ExitCode::ReturnContract,
        // InstructionResult::ReturnContractInNotInitEOF => ExitCode::ReturnContractInNotInitEOF,
        // InstructionResult::EOFOpcodeDisabledInLegacy => ExitCode::EOFOpcodeDisabledInLegacy,
        // InstructionResult::EOFFunctionStackOverflow => ExitCode::EOFFunctionStackOverflow,
        _ => ExitCode::UnknownError,
    }
}

// pub(crate) struct InputHelper {
//     input: Bytes,
// }
//
// impl InputHelper {
//     pub(crate) fn new() -> Self {
//         let input_size = LowLevelSDK::input_size();
//         let mut input = vec![0u8; input_size as usize];
//         LowLevelSDK::read(input.as_mut_ptr(), input_size, 0);
//         Self {
//             input: input.into(),
//         }
//     }
//
//     pub(crate) fn decode_method_id(&self) -> u32 {
//         let mut method_id = 0u32;
//         <CoreInput<Bytes> as ICoreInput>::MethodId::decode_field_header(
//             &self.input,
//             &mut method_id,
//         );
//         method_id
//     }
//
//     pub(crate) fn decode_method_input<T: Encoder<T> + Default>(&self) -> T {
//         let mut core_input = T::default();
//         <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(&self.input, &mut
// core_input);         core_input
//     }
// }
