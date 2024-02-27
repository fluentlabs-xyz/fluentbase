use alloc::vec;
use byteorder::{BigEndian, ByteOrder};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, U256},
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::ExitCode;
use revm::{
    precompile::HashMap,
    primitives::{
        Account,
        AccountInfo,
        Address,
        Bytecode,
        EVMError,
        ExecutionResult,
        B256,
        LONDON,
    },
    Database,
    DatabaseCommit,
    EVM,
};

pub(crate) const ZKTRIE_CODESIZE_NONCE_FIELD: u32 = 1;
pub(crate) const ZKTRIE_BALANCE_FIELD: u32 = 2;
pub(crate) const ZKTRIE_ROOT_FIELD: u32 = 3;
pub(crate) const ZKTRIE_KECCAK_CODE_HASH_FIELD: u32 = 4;
pub(crate) const ZKTRIE_CODE_HASH_FIELD: u32 = 5;

pub struct TxDb {}

impl Database for TxDb {
    type Error = ExitCode;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let mut result = AccountInfo::default();
        let mut bytes32 = Bytes32::default();
        unsafe {
            core::ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
        }
        let mut code_size_nonce = Bytes32::default();
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_CODESIZE_NONCE_FIELD,
            code_size_nonce.as_mut_ptr(),
        );
        let _code_size = BigEndian::read_u64(&code_size_nonce[16..]);
        result.nonce = BigEndian::read_u64(&code_size_nonce[24..]);
        LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
            result.balance.as_le_slice_mut().as_mut_ptr()
        });
        // LowLevelSDK::zktrie_field(
        //     bytes32.as_ptr(),
        //     ZKTRIE_ROOT_FIELD,
        //     result.root.as_mut_ptr(),
        // );
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_KECCAK_CODE_HASH_FIELD,
            result.code_hash.as_mut_ptr(),
        );
        // LowLevelSDK::zktrie_field(
        //     bytes32.as_ptr(),
        //     ZKTRIE_CODE_HASH_FIELD,
        //     result.code_hash.as_mut_ptr(),
        // );
        Ok(Some(result))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let code_size = LowLevelSDK::preimage_size(code_hash.as_ptr());
        let mut code = vec![0u8; code_size as usize];
        LowLevelSDK::preimage_copy(code_hash.as_ptr(), code.as_mut_ptr(), code_size);
        Ok(Bytecode::new_raw(code.into()))
    }

    fn storage(&mut self, _address: Address, _index: U256) -> Result<U256, Self::Error> {
        Ok(U256::ZERO)
    }

    fn block_hash(&mut self, _number: U256) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}

impl DatabaseCommit for TxDb {
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        for (k, v) in changes.into_iter() {
            let mut values: [Bytes32; 5] = [Bytes32::default(); 5];
            let code_size =
                v.info.code.map(|v| v.len() as u64).unwrap_or_else(|| {
                    LowLevelSDK::preimage_size(v.info.code_hash.as_ptr()) as u64
                });
            BigEndian::write_u64(&mut values[0][16..], code_size);
            BigEndian::write_u64(&mut values[0][24..], v.info.nonce);
            values[1].copy_from_slice(&v.info.balance.to_be_bytes::<32>());
            v.info.balance;
            // values[2].copy_from_slice(v.info.root.as_slice());
            values[3].copy_from_slice(v.info.code_hash.as_slice());
            // values[4].copy_from_slice(v.info.poseidon_code_hash.as_slice());
            let mut key32 = Bytes32::default();
            unsafe {
                core::ptr::copy(k.as_ptr(), key32.as_mut_ptr(), 20);
            }
            LowLevelSDK::zktrie_update(&key32, 8, &values);
        }
    }
}

fn execute_transaction() -> Result<(), ExitCode> {
    let mut evm = EVM::new();
    let mut db = TxDb {};
    evm.env.cfg.chain_id = ExecutionContext::env_chain_id();
    evm.env.cfg.spec_id = LONDON;
    // evm.env.block;
    // evm.env.tx;
    evm.database(&mut db);
    let result = evm.transact().map_err(|err| match err {
        EVMError::Database(exit_code) => exit_code,
        _ => ExitCode::TransactError,
    })?;
    match result.result {
        ExecutionResult::Success { .. } => {}
        ExecutionResult::Revert { .. } => {}
        ExecutionResult::Halt { .. } => {}
    }
    Ok(())
}

// #[no_mangle]
// pub fn _evm_call() {
//     // ...
// }
//
// #[no_mangle]
// pub fn _evm_exec_tx() -> i32 {
//     let exit_code = match execute_transaction() {
//         Ok(_) => ExitCode::Ok,
//         Err(err) => err,
//     };
//     exit_code.into_i32()
// }
