use crate::fluent_host::FluentHost;
use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use core::mem::take;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    AccountManager,
    ContextReader,
    ContractInput,
    CoreInput,
    EvmCallMethodInput,
    EvmCreateMethodInput,
    ICoreInput,
    LowLevelSDK,
    SharedAPI,
};
use fluentbase_types::{
    create_sovereign_import_linker,
    Address,
    Bytes,
    ExitCode,
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
    ($msg:tt) => {{
        use fluentbase_sdk::SovereignAPI;
        fluentbase_sdk::LowLevelSDK::debug_log($msg.as_ptr(), $msg.len() as u32);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!(msg);
    }};
}
#[cfg(not(feature = "e2e"))]
#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{}};
    ($($arg:tt)*) => {{}};
}

fn contract_context_from_call_inputs<CR: ContextReader>(
    cr: &CR,
    call_inputs: &Box<CallInputs>,
) -> ContractInput {
    ContractInput {
        contract_gas_limit: call_inputs.gas_limit,
        contract_address: call_inputs.target_address,
        contract_caller: call_inputs.caller,
        contract_value: call_inputs.value.get(),
        contract_is_static: call_inputs.is_static,
        block_chain_id: cr.block_chain_id(),
        block_coinbase: cr.block_coinbase(),
        block_timestamp: cr.block_timestamp(),
        block_number: cr.block_number(),
        block_difficulty: cr.block_difficulty(),
        block_prevrandao: cr.block_prevrandao(),
        block_gas_limit: cr.block_gas_limit(),
        block_base_fee: cr.block_base_fee(),
        tx_gas_limit: cr.tx_gas_limit(),
        tx_nonce: cr.tx_nonce(),
        tx_gas_price: cr.tx_gas_price(),
        tx_gas_priority_fee: cr.tx_gas_priority_fee(),
        tx_caller: cr.tx_caller(),
        tx_access_list: cr.tx_access_list(),
        tx_blob_hashes: cr.tx_blob_hashes(),
        tx_max_fee_per_blob_gas: cr.tx_max_fee_per_blob_gas(),
    }
}

fn contract_context_from_create_inputs<CR: ContextReader>(
    cr: &CR,
    create_inputs: &Box<CreateInputs>,
) -> ContractInput {
    ContractInput {
        contract_gas_limit: create_inputs.gas_limit,
        contract_address: Address::ZERO,
        contract_caller: create_inputs.caller,
        contract_value: create_inputs.value,
        contract_is_static: false,
        block_chain_id: cr.block_chain_id(),
        block_coinbase: cr.block_coinbase(),
        block_timestamp: cr.block_timestamp(),
        block_number: cr.block_number(),
        block_difficulty: cr.block_difficulty(),
        block_prevrandao: cr.block_prevrandao(),
        block_gas_limit: cr.block_gas_limit(),
        block_base_fee: cr.block_base_fee(),
        tx_gas_limit: cr.tx_gas_limit(),
        tx_nonce: cr.tx_nonce(),
        tx_gas_price: cr.tx_gas_price(),
        tx_gas_priority_fee: cr.tx_gas_priority_fee(),
        tx_caller: cr.tx_caller(),
        tx_access_list: cr.tx_access_list(),
        tx_blob_hashes: cr.tx_blob_hashes(),
        tx_max_fee_per_blob_gas: cr.tx_max_fee_per_blob_gas(),
    }
}

#[cfg(feature = "ecl")]
fn exec_evm_create<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    inputs: Box<CreateInputs>,
    depth: u32,
) -> CreateOutcome {
    // calc create input
    let contract_input = contract_context_from_create_inputs(cr, &inputs);
    let method_data = EvmCreateMethodInput {
        value: inputs.value,
        bytecode: inputs.init_code,
        gas_limit: inputs.gas_limit,
        salt: match inputs.scheme {
            CreateScheme::Create2 { salt } => Some(salt),
            CreateScheme::Create => None,
        },
        depth,
    };
    let create_output = crate::loader::_loader_create(&contract_input, am, method_data);

    let mut gas = Gas::new(create_output.gas);
    gas.record_refund(create_output.gas_refund);

    CreateOutcome {
        result: InterpreterResult {
            result: evm_error_from_exit_code(create_output.exit_code.into()),
            output: create_output.output,
            gas,
        },
        address: create_output.address,
    }
}

#[cfg(feature = "ecl")]
fn exec_evm_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    mut inputs: Box<CallInputs>,
    depth: u32,
) -> CallOutcome {
    let return_memory_offset = inputs.return_memory_offset.clone();

    let contract_input = contract_context_from_call_inputs(cr, &inputs);
    let method_data = EvmCallMethodInput {
        callee: inputs.bytecode_address,
        // here we take transfer value, because for DELEGATECALL it's not apparent
        value: inputs.value.transfer().unwrap_or_default(),
        input: take(&mut inputs.input),
        gas_limit: inputs.gas_limit,
        depth,
    };
    let call_output = crate::loader::_loader_call(&contract_input, am, method_data);

    // let core_input = CoreInput {
    //     method_id: EVM_CALL_METHOD_ID,
    //     method_data,
    // };
    // let mut gas_limit = inputs.gas_limit as u32;
    // let contract_input =
    //     contract_input_from_call_inputs(cr, &inputs, core_input.encode_to_vec(0).into())
    //         .encode_to_vec(0);
    // let (callee, _) = am.account(ECL_CONTRACT_ADDRESS);
    // let (output_buffer, exit_code) = am.exec_hash(
    //     callee.rwasm_code_hash.as_ptr(),
    //     &contract_input,
    //     &mut gas_limit as *mut u32,
    //     STATE_MAIN,
    // );
    // let call_output = if exit_code == 0 {
    //     let mut buffer_decoder = BufferDecoder::new(&output_buffer);
    //     let mut method_output = EvmCallMethodOutput::default();
    //     EvmCallMethodOutput::decode_body(&mut buffer_decoder, 0, &mut method_output);
    //     method_output
    // } else {
    //     EvmCallMethodOutput::from_exit_code(exit_code.into())
    // };

    let mut gas = Gas::new(call_output.gas_remaining);
    gas.record_refund(call_output.gas_refund);

    let interpreter_result = InterpreterResult {
        result: evm_error_from_exit_code(call_output.exit_code.into()),
        output: call_output.output.into(),
        gas,
    };

    CallOutcome {
        result: interpreter_result,
        memory_offset: return_memory_offset,
    }
}

#[cfg(feature = "ecl")]
pub(crate) fn exec_evm_bytecode<CR: ContextReader, AM: AccountManager>(
    mut cr: &CR,
    mut am: &AM,
    contract: Contract,
    gas_limit: u64,
    is_static: bool,
    depth: u32,
) -> InterpreterResult {
    debug_log!(
        "ecl(exec_evm_bytecode): executing EVM contract={}, caller={}, gas_limit={} bytecode={} input={} depth={}",
        &contract.target_address,
        &contract.caller,
        gas_limit,
        hex::encode(contract.bytecode.original_byte_slice()),
        hex::encode(&contract.input),
        depth,
    );
    if depth >= 1024 {
        debug_log!("depth limit reached: {}", depth);
    }
    let contract_address = contract.target_address;

    let instruction_table = make_instruction_table::<FluentHost<CR, AM>, CancunSpec>();

    let mut interpreter = Interpreter::new(contract, gas_limit, is_static);
    let mut host = FluentHost::new(cr, am);
    let mut shared_memory = SharedMemory::new();

    loop {
        // run EVM bytecode to produce next action
        let next_action = interpreter.run(shared_memory, &instruction_table, &mut host);

        // take memory and cr from interpreter and host back (return later)
        shared_memory = interpreter.take_memory();

        // take cr/am
        cr = host.cr.take().unwrap();
        am = host.am.take().unwrap();

        match next_action {
            InterpreterAction::Call { inputs } => {
                debug_log!(
                    "ecl(exec_evm_bytecode): nested call={:?} code={} caller={} callee={} gas={} prev_address={} value={} apparent_value={}",
                    inputs.scheme,
                    &inputs.bytecode_address,
                    &inputs.caller,
                    &inputs.target_address,
                    inputs.gas_limit,
                    contract_address,
                    hex::encode(inputs.value.transfer().unwrap_or_default().to_be_bytes::<32>()),
                    hex::encode(inputs.value.apparent().unwrap_or_default().to_be_bytes::<32>()),
                );
                let call_outcome = exec_evm_call(cr, am, inputs, depth + 1);
                interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
            }
            InterpreterAction::Create { inputs } => {
                debug_log!(
                    "ecl(exec_evm_bytecode): nested create caller={}, value={}",
                    inputs.caller,
                    hex::encode(inputs.value.to_be_bytes::<32>())
                );
                let create_outcome = exec_evm_create(cr, am, inputs, depth + 1);
                interpreter.insert_create_outcome(create_outcome);
            }
            InterpreterAction::Return { result } => {
                debug_log!(
                    "ecl(exec_evm_bytecode): return result={:?}, message={} gas_spent={}",
                    result.result,
                    hex::encode(result.output.as_ref()),
                    result.gas.spend(),
                );
                return result;
            }
            InterpreterAction::None => unreachable!("not supported EVM interpreter state"),
            InterpreterAction::EOFCreate { .. } => {
                unreachable!("not supported EVM interpreter state: EOF")
            }
        }

        // move cr/am back
        host.cr = Some(cr);
        host.am = Some(am);
    }
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

pub(crate) struct InputHelper {
    input: Bytes,
}

impl InputHelper {
    pub(crate) fn new() -> Self {
        let input_size = LowLevelSDK::input_size();
        let mut input = vec![0u8; input_size as usize];
        LowLevelSDK::read(&mut input, 0);
        Self {
            input: input.into(),
        }
    }

    pub(crate) fn decode_method_id(&self) -> u32 {
        let mut method_id = 0u32;
        <CoreInput<Bytes> as ICoreInput>::MethodId::decode_field_header(
            &self.input,
            &mut method_id,
        );
        method_id
    }

    pub(crate) fn decode_method_input<T: Encoder<T> + Default>(&self) -> T {
        let mut core_input = T::default();
        <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(&self.input, &mut core_input);
        core_input
    }
}
