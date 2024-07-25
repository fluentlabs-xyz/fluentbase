use crate::{Account, AccountStatus, ExitCode, Fuel, JournalCheckpoint, SharedAPI, F254, U256};
use alloc::vec::Vec;
use alloy_primitives::{Address, Bytes, B256};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use hashbrown::HashMap;

pub type IsColdAccess = bool;

pub struct BlockContext {
    pub chain_id: U256,
    pub coinbase: Address,
    pub timestamp: u64,
    pub number: u64,
    pub difficulty: u64,
    pub prev_randao: U256,
    pub gas_limit: u64,
}

pub struct TxContext {
    pub gas_limit: u64,
    pub nonce: u64,
    pub gas_price: U256,
    pub origin: Address,
    pub data: Bytes,
}

pub struct ContractContext {
    pub gas_limit: u64,
    pub address: Address,
    pub caller: Address,
    pub is_static: bool,
    pub value: U256,
}

pub struct TransitStateInput {
    pub accounts: HashMap<Address, Account>,
    pub preimages: HashMap<B256, Bytes>,
    pub block: BlockContext,
    pub transaction: TxContext,
}
pub struct TransitStateOutput {
    pub new_accounts: Vec<(Address, Account)>,
    pub new_preimages: Vec<(B256, Bytes)>,
    pub status: bool,
    pub gas_consumed: u64,
}

pub struct ContractCallInput {}
pub struct ContractCallOutput {}

#[derive(Clone, RlpEncodable, RlpDecodable)]
pub struct DelegatedExecution {
    pub address: Address,
    pub hash: F254,
    pub input: Bytes,
    pub fuel: u32,
}

impl DelegatedExecution {
    pub fn to_bytes(&self) -> Bytes {
        use alloy_rlp::Encodable;
        let mut buffer = Vec::new();
        self.encode(&mut buffer);
        buffer.into()
    }
    pub fn from_bytes(buffer: Bytes) -> Self {
        use alloy_rlp::Decodable;
        let mut buffer_slice = buffer.as_ref();
        Self::decode(&mut buffer_slice).expect("failed to decode delegated execution")
    }
}

// 1. Finish SovereignJournalAPI
// 2. Finalize Inputs/Outputs for state transition (EVM, SVM, FVM - upper structs)
// 3. Adapt EVM to the new API

pub trait SovereignJournalAPI {
    fn new<SDK: SharedAPI>(sdk: &SDK) -> Self;
    fn checkpoint(&self) -> JournalCheckpoint;
    fn commit<SDK: SharedAPI>(&mut self, sdk: &SDK);
    fn rollback(&mut self, checkpoint: JournalCheckpoint);

    fn write_account(&mut self, account: Account, status: AccountStatus);
    fn account(&self, address: &Address) -> (&Account, IsColdAccess);
    fn account_committed(&self, address: &Address) -> (&Account, IsColdAccess);

    fn write_preimage(&mut self, hash: B256, preimage: Bytes);
    fn preimage(&self, hash: &B256) -> Option<&[u8]>;
    fn preimage_size(&self, hash: &B256) -> u32;

    fn write_storage(&mut self, address: Address, slot: U256, value: U256) -> IsColdAccess;
    fn storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess);
    fn committed_storage(&self, address: Address, slot: U256) -> (U256, IsColdAccess);

    fn write_log(&mut self, address: Address, data: Bytes, topics: &[B256]);

    fn context_call<SDK: SharedAPI>(
        &mut self,
        sdk: &SDK,
        caller: Address,
        address: Address,
        value: U256,
        fuel: &mut Fuel,
        input: &[u8],
    ) -> (Bytes, ExitCode);
}

pub trait SharedJournalAPI {
    fn account(&self, address: Address) -> Account;
    fn transfer(&self, from: &mut Account, to: &mut Account, amount: U256);
    fn write_storage(&self, slot: U256, value: U256);
    fn storage(&self, slot: U256) -> U256;
    fn write_log(&self, data: Bytes, topics: &[B256]);
    fn call(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode);
    fn delegate(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode);
}
