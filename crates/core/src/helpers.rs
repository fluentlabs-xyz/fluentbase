use crate::account_types::JZKT_ACCOUNT_BALANCE_FIELD;
use crate::evm::call::_evm_call;
use crate::evm::create::_evm_create;
use crate::fluent_host::FluentHost;
use alloc::boxed::Box;
use alloc::string::ToString;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::evm::Bytes;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    Bytes32, EvmCallMethodInput, EvmCreateMethodInput, LowLevelAPI, LowLevelSDK,
};
use fluentbase_types::{Address, ExitCode, B256, STATE_DEPLOY, STATE_MAIN, U256};
use revm_interpreter::opcode::make_instruction_table;
use revm_interpreter::{Contract, Interpreter, InterpreterAction, SharedMemory};
use revm_primitives::CreateScheme;
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
    nonce.encode(&mut &mut out[22..]);
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
pub(crate) fn calc_storage_key(address: &Address, slot32_offset: *const u8) -> [u8; 32] {
    let mut slot0: [u8; 32] = [0u8; 32];
    let mut slot1: [u8; 32] = [0u8; 32];
    // split slot32 into two 16 byte values (slot is always 32 bytes)
    unsafe {
        core::ptr::copy(slot32_offset.offset(0), slot0.as_mut_ptr(), 16);
        core::ptr::copy(slot32_offset.offset(16), slot1.as_mut_ptr(), 16);
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

pub(crate) fn exec_evm_bytecode(
    contract: Contract,
    gas_limit: u64,
    is_static: bool,
) -> Result<Bytes, ExitCode> {
    let mut interpreter = Interpreter::new(Box::new(contract), gas_limit, is_static);
    let instruction_table = make_instruction_table::<FluentHost, DefaultEvmSpec>();
    let mut host = FluentHost::default();
    let shared_memory = SharedMemory::new();
    match interpreter.run(shared_memory, &instruction_table, &mut host) {
        InterpreterAction::Call { inputs } => {
            match _evm_call(EvmCallMethodInput {
                callee: inputs.contract,
                value: inputs.transfer.value,
                input: inputs.input,
                gas_limit: inputs.gas_limit,
            }) {
                Ok(result) => {
                    return Ok(result);
                }
                Err(exit_code) => {
                    panic!(
                        "EVM nested call failed with error: {} ({})",
                        exit_code as i32, exit_code
                    );
                }
            }
        }
        InterpreterAction::Create { inputs } => {
            let result = match inputs.scheme {
                CreateScheme::Create => _evm_create(EvmCreateMethodInput {
                    value: inputs.value,
                    init_code: inputs.init_code,
                    gas_limit: inputs.gas_limit,
                    salt: None,
                }),
                CreateScheme::Create2 { salt } => _evm_create(EvmCreateMethodInput {
                    value: inputs.value,
                    init_code: inputs.init_code,
                    gas_limit: inputs.gas_limit,
                    salt: Some(salt),
                }),
            };
            match result {
                Ok(result) => {
                    return Ok(result.into_array().into());
                }
                Err(exit_code) => {
                    panic!(
                        "EVM nested created failed with error: {} ({})",
                        exit_code as i32, exit_code
                    );
                }
            }
        }
        InterpreterAction::Return { result } => {
            if result.is_revert() {
                LowLevelSDK::sys_write(&result.output);
                return Err(ExitCode::EVMCallRevert);
            } else if result.is_error() {
                LowLevelSDK::sys_write(&result.output);
                return Err(ExitCode::EVMCallError);
            }
            return Ok(result.output);
        }
        InterpreterAction::None => return Ok(Bytes::default()),
    };
}

#[inline(always)]
pub(crate) fn unwrap_exit_code<T>(result: Result<T, ExitCode>) -> T {
    result.unwrap_or_else(|exit_code| {
        LowLevelSDK::sys_halt(exit_code.into_i32());
        panic!("execution halted: {exit_code}")
    })
}

#[cfg(test)]
mod tests {
    use revm_primitives::b256;

    use fluentbase_types::address;

    use super::*;

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
