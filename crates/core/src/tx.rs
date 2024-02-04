use fluentbase_sdk::{evm::U256, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;
use revm::{
    precompile::HashMap,
    primitives::{Account, AccountInfo, Address, Bytecode, B256},
    Database,
    DatabaseCommit,
    EVM,
};

pub(crate) const ZKTRIE_CODESIZE_NONCE_FIELD: u32 = 1;
pub(crate) const ZKTRIE_BALANCE_FIELD: u32 = 2;
pub(crate) const ZKTRIE_ROOT_FIELD: u32 = 3;
pub(crate) const ZKTRIE_KECCAK_CODE_HASH_FIELD: u32 = 4;
pub(crate) const ZKTRIE_CODE_HASH_FIELD: u32 = 5;

struct EvmTxDb {}

impl Database for EvmTxDb {
    type Error = ExitCode;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        todo!()
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // address(20) + index(11) = 31 (value fits into bn254)
        if index.byte_len() > 11 {
            return Err(ExitCode::StorageSlotOverflow);
        }
        // LowLevelSDK::zktrie_field();
        Ok(U256::ZERO)
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
        todo!()
    }
}

impl DatabaseCommit for EvmTxDb {
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        todo!()
    }
}

fn execute_transaction() -> Result<(), ExitCode> {
    let mut evm = EVM::new();
    let mut db = EvmTxDb {};

    evm.database(&mut db);
    let result = evm.transact()?;

    Ok(())
}

#[no_mangle]
pub fn _evm_exec_tx() -> i32 {
    let exit_code = match execute_transaction() {
        Ok(_) => ExitCode::Ok,
        Err(err) => err,
    };
    exit_code.into_i32()
}
