use crate::{
    alloc_slice,
    Account,
    AccountStatus,
    ExitCode,
    Fuel,
    JournalCheckpoint,
    SharedAPI,
    U256,
};
use alloy_primitives::{Address, Bytes, B256};
use hashbrown::HashMap;

pub type IsColdAccess = bool;

pub struct BlockContext {
    chain_id: U256,
    coinbase: Address,
    timestamp: u64,
    number: u64,
    difficulty: u64,
    prev_randao: U256,
    gas_limit: u64,
}

pub struct TxContext {
    gas_limit: u64,
    nonce: u64,
    gas_price: U256,
    origin: Address,
}

pub struct ContractContext {
    gas_limit: u64,
    address: Address,
    caller: Address,
    is_static: bool,
    value: U256,
}

pub struct SovereignInput {
    accounts: HashMap<Address, Account>,
    precompiles: HashMap<B256, Bytes>,
    block: BlockContext,
    transaction: TxContext,
}
pub struct SovereignOutput {}

pub trait SovereignJournalAPI {
    fn new<SDK: SharedAPI>(sdk: &SDK) -> Self;
    fn checkpoint(&self) -> JournalCheckpoint;
    fn commit<SDK: SharedAPI>(&mut self, sdk: &SDK);
    fn rollback(&mut self, checkpoint: JournalCheckpoint);

    fn write_account(&mut self, account: Account, status: AccountStatus);
    fn account(&self, address: &Address) -> (&Account, IsColdAccess);

    fn write_preimage(&mut self, hash: B256, preimage: Bytes);
    fn preimage_size(&self, hash: &B256) -> u32;
    fn preimage(&self, hash: &B256) -> Option<&[u8]>;

    fn write_storage(&mut self, address: Address, slot: U256, value: U256) -> IsColdAccess;
    fn storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess);
    fn committed_storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess);

    fn write_log(&mut self, address: Address, data: Bytes, topics: &[B256]);

    fn system_call(&mut self, address: Address, input: &[u8], fuel: &mut Fuel)
        -> (Bytes, ExitCode);
    fn context_call(
        &mut self,
        address: Address,
        input: &[u8],
        context: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> (Bytes, ExitCode);
}

pub trait SharedJournalAPI {
    fn account(&self, address: Address) -> Account;
    fn transfer(&self, from: &mut Account, to: &mut Account, amount: U256);
    fn write_storage(&self, slot: U256, value: U256);
    fn storage(&self, slot: U256) -> U256;
    fn write_log(&self, data: Bytes, topics: &[B256]);
}
