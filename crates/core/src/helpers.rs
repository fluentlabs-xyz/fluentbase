#[cfg(feature = "ecl")]
use crate::evm::{call::_evm_call, create::_evm_create};
use crate::fluent_host::FluentHost;
use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use core::marker::PhantomData;
use core::mem::take;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    AccountManager, ContextReader, ContractInput, CoreInput, EvmCallMethodInput,
    EvmCreateMethodInput, ICoreInput, LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::SysFuncIdx::SYS_STATE;
use fluentbase_types::{
    create_sovereign_import_linker, Address, Bytes, Bytes32, ExitCode, B256, STATE_DEPLOY,
    STATE_MAIN, U256,
};
use revm_interpreter::{
    opcode::make_instruction_table, CallInputs, CallOutcome, Contract, CreateInputs, CreateOutcome,
    Gas, InstructionResult, Interpreter, InterpreterAction, InterpreterResult, SharedMemory,
};
use revm_primitives::{CancunSpec, CreateScheme};
use rwasm::engine::bytecode::Instruction;
use rwasm::engine::{RwasmConfig, StateRouterConfig};
use rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule};

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
pub fn calc_create_address(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    Encodable::encode(&nonce, &mut &mut out[22..]);
    let mut hash = B256::ZERO;
    let out = &out[..len];
    LowLevelSDK::crypto_keccak256(out.as_ptr(), out.len() as u32, hash.as_mut_ptr());
    Address::from_word(hash)
}

#[inline(always)]
pub fn calc_create2_address(deployer: &Address, salt: &U256, init_code_hash: &B256) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    LowLevelSDK::crypto_keccak256(bytes.as_ptr(), bytes.len() as u32, bytes.as_mut_ptr());
    let bytes32: Bytes32 = bytes[0..32].try_into().unwrap();
    Address::from_word(B256::from(bytes32))
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
            opcode: Instruction::Call(SYS_STATE.into()),
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

#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{
        fluentbase_sdk::LowLevelSDK::debug_log($msg.as_ptr(), $msg.len() as u32);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        fluentbase_sdk::LowLevelSDK::debug_log(msg.as_ptr(), msg.len() as u32);
    }};
}

const DOMAIN: [u8; 32] = [0u8; 32];

#[inline(always)]
pub fn calc_storage_key(address: &Address, slot32_le_ptr: *const u8) -> [u8; 32] {
    let mut slot0: [u8; 32] = [0u8; 32];
    let mut slot1: [u8; 32] = [0u8; 32];
    // split slot32 into two 16 byte values (slot is always 32 bytes)
    unsafe {
        core::ptr::copy(slot32_le_ptr.offset(0), slot0.as_mut_ptr(), 16);
        core::ptr::copy(slot32_le_ptr.offset(16), slot1.as_mut_ptr(), 16);
    }
    // pad address to 32 bytes value (11 bytes to avoid 254 overflow)
    let mut address32: [u8; 32] = [0u8; 32];
    address32[11..31].copy_from_slice(address.as_slice());
    // compute a storage key, where formula is `p(address, p(slot_0, slot_1))`
    let mut storage_key: [u8; 32] = [0u8; 32];
    LowLevelSDK::crypto_poseidon2(
        slot0.as_ptr(),
        slot1.as_ptr(),
        DOMAIN.as_ptr(),
        storage_key.as_mut_ptr(),
    );
    LowLevelSDK::crypto_poseidon2(
        address32.as_ptr(),
        storage_key.as_ptr(),
        DOMAIN.as_ptr(),
        storage_key.as_mut_ptr(),
    );
    storage_key
}

fn contract_input_from_call_inputs<CR: ContextReader>(
    cr: &CR,
    call_inputs: &Box<CallInputs>,
    input: Bytes,
) -> ContractInput {
    ContractInput {
        journal_checkpoint: cr.journal_checkpoint(),
        contract_gas_limit: call_inputs.gas_limit,
        contract_address: call_inputs.context.address,
        contract_caller: call_inputs.context.caller,
        contract_input: input,
        contract_value: call_inputs.context.apparent_value,
        contract_is_static: call_inputs.is_static,
        block_chain_id: cr.block_chain_id(),
        block_coinbase: cr.block_coinbase(),
        block_timestamp: cr.block_timestamp(),
        block_number: cr.block_number(),
        block_difficulty: cr.block_difficulty(),
        block_gas_limit: cr.block_gas_limit(),
        block_base_fee: cr.block_base_fee(),
        tx_gas_limit: cr.tx_gas_limit(),
        tx_nonce: cr.tx_nonce(),
        tx_gas_price: cr.tx_gas_price(),
        tx_gas_priority_fee: cr.tx_gas_priority_fee(),
        tx_caller: cr.tx_caller(),
        tx_access_list: cr.tx_access_list(),
    }
}

fn contract_input_from_create_inputs<CR: ContextReader>(
    cr: &CR,
    create_inputs: &Box<CreateInputs>,
    input: Bytes,
) -> ContractInput {
    ContractInput {
        journal_checkpoint: cr.journal_checkpoint(),
        contract_gas_limit: create_inputs.gas_limit,
        contract_address: Address::ZERO,
        contract_caller: create_inputs.caller,
        contract_input: input,
        contract_value: create_inputs.value,
        contract_is_static: false,
        block_chain_id: cr.block_chain_id(),
        block_coinbase: cr.block_coinbase(),
        block_timestamp: cr.block_timestamp(),
        block_number: cr.block_number(),
        block_difficulty: cr.block_difficulty(),
        block_gas_limit: cr.block_gas_limit(),
        block_base_fee: cr.block_base_fee(),
        tx_gas_limit: cr.tx_gas_limit(),
        tx_nonce: cr.tx_nonce(),
        tx_gas_price: cr.tx_gas_price(),
        tx_gas_priority_fee: cr.tx_gas_priority_fee(),
        tx_caller: cr.tx_caller(),
        tx_access_list: cr.tx_access_list(),
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
    let contract_input = contract_input_from_create_inputs(cr, &inputs, Bytes::new());
    let create_input = EvmCreateMethodInput {
        value: inputs.value,
        bytecode: inputs.init_code,
        gas_limit: inputs.gas_limit,
        salt: match inputs.scheme {
            CreateScheme::Create2 { salt } => Some(salt),
            CreateScheme::Create => None,
        },
        depth: depth + 1,
    };

    let create_output = _evm_create(&contract_input, am, create_input);

    let mut gas = Gas::new(create_output.gas);
    gas.record_refund(create_output.gas_refund);

    CreateOutcome {
        result: InterpreterResult {
            result: evm_error_from_exit_code(create_output.exit_code.into()),
            output: create_output.output,
            gas,
        },
        address: None,
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

    let contract_input = contract_input_from_call_inputs(cr, &inputs, Bytes::new());
    let call_output = _evm_call(
        &contract_input,
        am,
        EvmCallMethodInput {
            callee: inputs.contract,
            // here we take transfer value, because for DELEGATECALL it's not apparent
            value: inputs.transfer.value,
            input: take(&mut inputs.input),
            gas_limit: inputs.gas_limit,
            depth: depth + 1,
        },
    );

    // let core_input = CoreInput {
    //     method_id: EVM_CALL_METHOD_ID,
    //     method_data: EvmCallMethodInput {
    //         callee: inputs.contract,
    //         value: inputs.context.apparent_value,
    //         input: take(&mut inputs.input),
    //         gas_limit: inputs.gas_limit,
    //     },
    // };
    // let mut gas_limit = inputs.gas_limit as u32 * EVM_GAS_MULTIPLIER as u32;
    // let contract_input =
    //     contract_input_from_call_inputs(cr, inputs, core_input.encode_to_vec(0).into()).encode_to_vec(0);
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
    //     EvmCallMethodOutput::from_exit_code(exit_code.into()).with_gas(0)
    // };

    let mut gas = Gas::new(call_output.gas);
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
        "ecl(exec_evm_bytecode): executing EVM contract={}, caller={}, gas_limit={} bytecode={}",
        &contract.address,
        &contract.caller,
        gas_limit,
        hex::encode(contract.bytecode.original_bytecode_slice()),
    );
    let contract_address = contract.address;

    let instruction_table = make_instruction_table::<FluentHost<CR, AM>, CancunSpec>();

    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit, is_static);
    let mut host = FluentHost::new(cr, am);
    let mut shared_memory = SharedMemory::new();

    loop {
        // run EVM bytecode to produce next action
        let next_action = interpreter.run(shared_memory, &instruction_table, &mut host);

        // take memory and cr from interpreter and host back (return later)
        shared_memory = interpreter.take_memory();

        cr = host.cr.take().unwrap();
        am = host.am.take().unwrap();

        match next_action {
            InterpreterAction::Call { inputs } => {
                debug_log!(
                    "ecl(exec_evm_bytecode): nested call={:?} code={} caller={} callee={} address={} gas={} prev_address={}",
                    inputs.context.scheme,
                    &inputs.context.code_address,
                    &inputs.context.caller,
                    &inputs.contract,
                    &inputs.context.address,
                    inputs.gas_limit,
                    contract_address,
                );
                interpreter
                    .insert_call_outcome(&mut shared_memory, exec_evm_call(cr, am, inputs, depth))
            }
            InterpreterAction::Create { inputs } => {
                debug_log!("ecl(exec_evm_bytecode): nested create");
                interpreter.insert_create_outcome(exec_evm_create(cr, am, inputs, depth))
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
        }

        // move cr back
        host.cr = Some(cr);
        host.am = Some(am);
    }
}

pub(crate) fn evm_error_from_exit_code(exit_code: ExitCode) -> InstructionResult {
    match exit_code {
        ExitCode::Ok => InstructionResult::Stop,
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::CallDepthOverflow => InstructionResult::CallTooDeep,
        ExitCode::InsufficientBalance => InstructionResult::OutOfFunds,
        ExitCode::OutOfFuel => InstructionResult::OutOfGas,
        ExitCode::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        ExitCode::WriteProtection => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::InvalidEfOpcode => InstructionResult::InvalidFEOpcode,
        ExitCode::InvalidJump => InstructionResult::InvalidJump,
        ExitCode::NotActivatedEIP => InstructionResult::NotActivated,
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
        // TODO(dmitry123): "what's proper unknown error code mapping?"
        _ => InstructionResult::OutOfGas,
    }
}

pub(crate) fn exit_code_from_evm_error(evm_error: InstructionResult) -> ExitCode {
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
        | InstructionResult::InvalidOperandOOG => ExitCode::OutOfFuel,
        InstructionResult::OpcodeNotFound => ExitCode::OpcodeNotFound,
        InstructionResult::CallNotAllowedInsideStatic
        | InstructionResult::StateChangeDuringStaticCall => ExitCode::WriteProtection,
        InstructionResult::InvalidFEOpcode => ExitCode::InvalidEfOpcode,
        InstructionResult::InvalidJump => ExitCode::InvalidJump,
        InstructionResult::NotActivated => ExitCode::NotActivatedEIP,
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
    }
}

#[inline(always)]
pub(crate) fn unwrap_exit_code<T>(result: Result<T, ExitCode>) -> T {
    result.unwrap_or_else(|v| {
        debug_log!("unwrap_exit_code: {}", v);
        LowLevelSDK::sys_halt(v.into_i32());
        // we can it for testing purposes, this branch never happens
        panic!("execution halted: {v}")
    })
}

pub(crate) struct InputHelper<CR: ContextReader> {
    input: Bytes,
    _phantom: PhantomData<CR>,
}

impl<CR: ContextReader> InputHelper<CR> {
    pub(crate) fn new(cr: CR) -> Self {
        Self {
            input: cr.contract_input(),
            _phantom: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::ExecutionContext;
    use fluentbase_types::address;
    use revm_interpreter::analysis::to_analysed;
    use revm_interpreter::BytecodeLocked;
    use revm_primitives::{b256, hex, Bytecode};

    #[test]
    fn test_create_address() {
        for (address, nonce) in [
            (address!("0000000000000000000000000000000000000000"), 0),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MIN,
            ),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MAX,
            ),
            (address!("2340820934820934820934809238402983400000"), 0),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MIN,
            ),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MAX,
            ),
        ] {
            assert_eq!(
                calc_create_address(&address, nonce as u64),
                address.create(nonce as u64)
            );
        }
    }

    #[test]
    fn test_create2_address() {
        let address = Address::ZERO;
        for (salt, hash) in [(
            b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
        )] {
            assert_eq!(
                calc_create2_address(&address, &salt.into(), &hash),
                address.create2(salt, hash)
            );
        }
    }
}
