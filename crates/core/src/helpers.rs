use crate::consts::ECL_CONTRACT_ADDRESS;
#[cfg(feature = "ecl")]
use crate::evm::{call::_evm_call, create::_evm_create};
use crate::{
    account_types::JZKT_ACCOUNT_BALANCE_FIELD, fluent_host::FluentHost, Account, AccountCheckpoint,
};
use alloc::{boxed::Box, string::ToString, vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use core::str::from_utf8;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput},
    Bytes32, CoreInput, EvmCallMethodInput, EvmCreateMethodInput, ICoreInput, LowLevelAPI,
    LowLevelSDK, EVM_CALL_METHOD_ID,
};
use fluentbase_types::{Address, Bytes, ExitCode, B256, STATE_DEPLOY, STATE_MAIN, U256};
use hashbrown::Equivalent;
use revm_interpreter::instructions::host::create;
use revm_interpreter::opcode::InstructionTable;
use revm_interpreter::{
    opcode::make_instruction_table, return_ok, CallInputs, CallOutcome, Contract, CreateInputs,
    CreateOutcome, Gas, InstructionResult, Interpreter, InterpreterAction, InterpreterResult,
    SharedMemory,
};
use revm_primitives::{CreateScheme, MAX_CODE_SIZE};
use rwasm::rwasm::BinaryFormat;

#[macro_export]
macro_rules! decode_method_input {
    ($core_input: ident, $method_input: ident) => {{
        let mut buffer = BufferDecoder::new(&mut $core_input.method_data);
        let mut method_input = $method_input::default();
        $method_input::decode_body(&mut buffer, 0, &mut method_input);
        method_input
    }};
}

pub type DefaultEvmSpec = revm_interpreter::primitives::ShanghaiSpec;

#[inline]
pub(crate) fn get_contract_input_offset_and_len() -> (u32, u32) {
    let mut header = [0u8; 8];
    LowLevelSDK::sys_read(
        &mut header,
        <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
    );
    let offset = LittleEndian::read_u32(&header[0..4]);
    let length = LittleEndian::read_u32(&header[4..8]);
    (offset, length)
}

#[inline(always)]
pub(crate) fn read_balance(address: Address, value: &mut U256) {
    let mut bytes32 = Bytes32::default();
    unsafe {
        core::ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::jzkt_get(bytes32.as_ptr(), JZKT_ACCOUNT_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
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
pub fn rwasm_exec_hash(code_hash32: &[u8], input: &[u8], gas_limit: u32, is_deploy: bool) -> i32 {
    LowLevelSDK::sys_exec_hash(
        code_hash32.as_ptr(),
        input.as_ptr(),
        input.len() as u32,
        core::ptr::null_mut(),
        0,
        &gas_limit as *const u32,
        if is_deploy { STATE_DEPLOY } else { STATE_MAIN },
    )
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

fn contract_input_from_call_inputs(
    gas_limit: u64,
    callee_address: Address,
    input: Bytes,
    value: U256,
    is_static: bool,
) -> Vec<u8> {
    ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint(),
        contract_gas_limit: gas_limit,
        contract_address: callee_address,
        contract_caller: ExecutionContext::contract_address(),
        contract_input: input,
        contract_value: value,
        contract_is_static: is_static,
        block_chain_id: ExecutionContext::block_chain_id(),
        block_coinbase: ExecutionContext::block_coinbase(),
        block_timestamp: ExecutionContext::block_timestamp(),
        block_number: ExecutionContext::block_number(),
        block_difficulty: ExecutionContext::block_difficulty(),
        block_gas_limit: ExecutionContext::block_gas_limit(),
        block_base_fee: ExecutionContext::block_base_fee(),
        tx_gas_limit: ExecutionContext::tx_gas_limit(),
        tx_nonce: ExecutionContext::tx_nonce(),
        tx_gas_price: ExecutionContext::tx_gas_price(),
        tx_gas_priority_fee: ExecutionContext::tx_gas_priority_fee(),
        tx_caller: ExecutionContext::tx_caller(),
        tx_access_list: ExecutionContext::tx_access_list(),
    }
    .encode_to_vec(0)
}

#[cfg(feature = "ecl")]
fn exec_evm_create(inputs: Box<CreateInputs>) -> CreateOutcome {
    // calc create input
    let create_input = EvmCreateMethodInput {
        value: inputs.value,
        init_code: inputs.init_code,
        gas_limit: inputs.gas_limit,
        salt: match inputs.scheme {
            CreateScheme::Create2 { salt } => Some(salt),
            CreateScheme::Create => None,
        },
    };

    let create_outcome = match _evm_create(create_input) {
        Ok(result) => CreateOutcome {
            result: InterpreterResult {
                result: InstructionResult::Continue,
                output: Default::default(),
                gas: Gas::new(inputs.gas_limit),
            },
            address: Some(result),
        },
        Err(exit_code) => CreateOutcome {
            result: InterpreterResult {
                result: match exit_code {
                    ExitCode::Ok => InstructionResult::Continue,
                    ExitCode::Panic => InstructionResult::Revert,
                    _ => InstructionResult::FatalExternalError,
                },
                output: Default::default(),
                gas: Gas::new(inputs.gas_limit),
            },
            address: None,
        },
    };

    create_outcome
}

#[cfg(feature = "ecl")]
fn exec_evm_call(inputs: Box<CallInputs>) -> CallOutcome {
    let return_memory_offset = inputs.return_memory_offset.clone();

    let core_input = CoreInput {
        method_id: EVM_CALL_METHOD_ID,
        method_data: EvmCallMethodInput {
            callee: inputs.contract,
            value: inputs.transfer.value,
            input: inputs.input,
            gas_limit: inputs.gas_limit,
        },
    };

    let mut gas_limit = inputs.gas_limit as u32;
    let contract_input = contract_input_from_call_inputs(
        inputs.gas_limit,
        inputs.contract,
        core_input.encode_to_vec(0).into(),
        inputs.transfer.value,
        inputs.is_static,
    );
    let callee = Account::new_from_jzkt(ECL_CONTRACT_ADDRESS);
    let exit_code = LowLevelSDK::sys_exec_hash(
        callee.rwasm_code_hash.as_ptr(),
        contract_input.as_ptr(),
        contract_input.len() as u32,
        core::ptr::null_mut(),
        0,
        &mut gas_limit as *mut u32,
        STATE_MAIN,
    );

    // read EVM call output
    let output_size = LowLevelSDK::sys_output_size();
    let mut output_buffer = vec![0u8; output_size as usize];
    LowLevelSDK::sys_read_output(output_buffer.as_mut_ptr(), 0, output_size);

    // convert exit code
    let exit_code = ExitCode::from(exit_code);

    let interpreter_result = InterpreterResult {
        result: match ExitCode::from(exit_code) {
            ExitCode::Ok => InstructionResult::Continue,
            ExitCode::Panic => InstructionResult::Revert,
            _ => InstructionResult::Revert,
        },
        output: output_buffer.into(),
        gas: Gas::new(gas_limit as u64),
    };

    CallOutcome {
        result: interpreter_result,
        memory_offset: return_memory_offset,
    }
}

#[cfg(feature = "ecl")]
pub(crate) fn exec_evm_bytecode(
    contract: Contract,
    gas_limit: u64,
    is_static: bool,
) -> InterpreterResult {
    use crate::evm::create::_evm_create;

    static INSTRUCTION_TABLE: InstructionTable<FluentHost> =
        make_instruction_table::<FluentHost, DefaultEvmSpec>();

    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit, is_static);
    let mut host = FluentHost::default();
    let mut shared_memory = SharedMemory::new();

    loop {
        // run EVM bytecode to produce next action
        let next_action = interpreter.run(shared_memory, &INSTRUCTION_TABLE, &mut host);

        // take memory from interpreter back
        shared_memory = interpreter.take_memory();

        match next_action {
            InterpreterAction::Call { inputs } => {
                interpreter.insert_call_outcome(&mut shared_memory, exec_evm_call(inputs))
            }
            InterpreterAction::Create { inputs } => {
                interpreter.insert_create_outcome(exec_evm_create(inputs))
            }
            InterpreterAction::Return { result } => {
                return result;
            }
            InterpreterAction::None => unreachable!("not supported EVM interpreter state"),
        }
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
    result.unwrap_or_else(|exit_code| {
        LowLevelSDK::sys_halt(exit_code.into_i32());
        panic!("execution halted: {exit_code}")
    })
}

pub(crate) struct InputHelper {
    input: Bytes,
}

impl InputHelper {
    pub(crate) fn new() -> Self {
        Self {
            input: ExecutionContext::contract_input(),
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
    use fluentbase_types::address;
    use revm_primitives::b256;

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
