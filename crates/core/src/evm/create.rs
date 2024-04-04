use crate::evm::{calc_create_address, read_input_address, Account, HostImpl, MAX_CODE_SIZE};
use alloc::{alloc::alloc, boxed::Box};
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput, U256},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    opcode::make_instruction_table,
    primitives::{Bytes, LondonSpec},
    Contract,
    Interpreter,
    SharedMemory,
};

#[no_mangle]
pub fn _evm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    output20_offset: *mut u8,
    gas_limit: u32,
) -> i32 {
    // TODO: "gas calculations"
    // TODO: "call depth stack check >= 1024"
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection.into_i32();
    }
    // read value input and contract address
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let contract_address =
        read_input_address(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    // load deployer and contract accounts
    let mut deployer = Account::read_account(&contract_address);
    let created_address = calc_create_address(&contract_address, deployer.nonce);
    let mut contract = Account::read_account(&created_address);
    // if nonce or code is not empty then its collision
    if contract.is_not_empty() {
        return ExitCode::CreateCollision.into_i32();
    }
    deployer.nonce += 1;
    contract.nonce = 1;
    // transfer value to the just created account
    if !deployer.transfer_value(&mut contract, &value) {
        return ExitCode::InsufficientBalance.into_i32();
    }
    // execute deployer bytecode
    let mut shared_memory = SharedMemory::new();
    let contract = Contract {
        input: Bytes::from_static(unsafe {
            &*ptr::slice_from_raw_parts(code_offset, code_length as usize)
        }),
        bytecode: Default::default(),
        hash: Default::default(),
        address: Default::default(),
        caller: Default::default(),
        value,
    };
    let mut evm = Interpreter::new(
        Box::new(contract),
        gas_limit as u64,
        false,
        &mut shared_memory,
    );
    let instruction_table = make_instruction_table::<HostImpl, LondonSpec>();
    let mut host = HostImpl::default();
    let result = evm.run(&instruction_table, &mut host);
    // read output bytecode
    let bytecode_length = LowLevelSDK::sys_output_size();
    if bytecode_length > MAX_CODE_SIZE {
        return ExitCode::ContractSizeLimit.into_i32();
    }
    let bytecode = unsafe {
        alloc(Layout::from_size_align_unchecked(
            bytecode_length as usize,
            8,
        ))
    };
    LowLevelSDK::sys_read_output(bytecode, 0, bytecode_length);
    // calc keccak256 and poseidon hashes for account
    // LowLevelSDK::crypto_keccak256(
    //     code_offset,
    //     code_length,
    //     contract.keccak_code_hash.as_mut_ptr(),
    // );
    // LowLevelSDK::crypto_poseidon(code_offset, code_length, contract.code_hash.as_mut_ptr());
    // commit account changes
    // contract.commit(&created_address);
    // copy result address to output and return ok
    unsafe { ptr::copy(created_address.as_ptr(), output20_offset, 20) }
    ExitCode::Ok.into_i32()
}

#[cfg(test)]
mod tests {
    use crate::evm::calc_create_address;
    use alloc::vec;
    use fluentbase_sdk::evm::Address;
    use fluentbase_types::{address, B256};
    use keccak_hash::keccak;

    #[test]
    fn create_correctness() {
        fn create_slow(address: &Address, nonce: u64) -> Address {
            use alloy_rlp::Encodable;
            let mut out = vec![];
            alloy_rlp::Header {
                list: true,
                payload_length: address.length() + nonce.length(),
            }
            .encode(&mut out);
            address.encode(&mut out);
            nonce.encode(&mut out);
            Address::from_word(keccak(out).0.into())
        }
        let tests = vec![(address!("0000000000000000000000000000000000000000"), 100)];
        for (address, nonce) in tests {
            assert_eq!(
                calc_create_address(&address, nonce),
                create_slow(&address, nonce)
            )
        }
    }

    #[test]
    fn create2_correctness() {
        let tests = [
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "00",
                "4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38",
            ),
            (
                "deadbeef00000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "00",
                "B928f69Bb1D91Cd65274e3c79d8986362984fDA3",
            ),
            (
                "deadbeef00000000000000000000000000000000",
                "000000000000000000000000feed000000000000000000000000000000000000",
                "00",
                "D04116cDd17beBE565EB2422F2497E06cC1C9833",
            ),
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "deadbeef",
                "70f2b2914A2a4b783FaEFb75f459A580616Fcb5e",
            ),
            (
                "00000000000000000000000000000000deadbeef",
                "00000000000000000000000000000000000000000000000000000000cafebabe",
                "deadbeef",
                "60f3f640a8508fC6a86d45DF051962668E1e8AC7",
            ),
            (
                "00000000000000000000000000000000deadbeef",
                "00000000000000000000000000000000000000000000000000000000cafebabe",
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                "1d8bfDC5D46DC4f61D6b6115972536eBE6A8854C",
            ),
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                "E33C0C7F7df4809055C3ebA6c09CFe4BaF1BD9e0",
            ),
        ];
        for (from, salt, init_code, expected) in tests {
            let from = from.parse::<Address>().unwrap();

            let salt = hex::decode(salt).unwrap();
            let salt: [u8; 32] = salt.try_into().unwrap();

            let init_code = hex::decode(init_code).unwrap();
            let init_code_hash: B256 = keccak(&init_code).0.into();

            let expected = expected.parse::<Address>().unwrap();

            assert_eq!(expected, from.create2(salt, init_code_hash));
            assert_eq!(expected, from.create2_from_code(salt, init_code));
        }
    }
}
