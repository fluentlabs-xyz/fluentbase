use crate::{Account, AccountStatus, Address, ExitCode, Fuel, JournalCheckpoint, B256, F254, U256};
use alloc::{vec, vec::Vec};
use alloy_primitives::Bytes;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use fluentbase_codec::Codec;
use hashbrown::HashMap;
use revm_primitives::Env;

/// A trait for providing shared API functionality.
pub trait NativeAPI {
    fn keccak256(&self, data: &[u8]) -> B256;
    fn sha256(&self, _data: &[u8]) -> B256 {
        unreachable!("sha256 is not supported yet")
    }
    fn poseidon(&self, data: &[u8]) -> F254;
    fn poseidon_hash(&self, fa: &F254, fb: &F254, fd: &F254) -> F254;
    fn ec_recover(&self, digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65];
    fn debug_log(&self, message: &str);

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;
    fn write(&self, value: &[u8]);
    fn forward_output(&self, offset: u32, len: u32);
    fn exit(&self, exit_code: i32) -> !;
    fn output_size(&self) -> u32;
    fn read_output(&self, target: &mut [u8], offset: u32);
    fn state(&self) -> u32;
    fn read_context(&self, target: &mut [u8], offset: u32);
    fn charge_fuel(&self, fuel: &mut Fuel);
    fn exec(
        &self,
        code_hash: &F254,
        address: &Address,
        input: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> i32;
    fn resume(&self, call_id: u32, exit_code: i32) -> i32;

    fn return_data(&self) -> Bytes {
        let output_size = self.output_size();
        let mut buffer = vec![0u8; output_size as usize];
        self.read_output(&mut buffer, 0);
        buffer.into()
    }
}

pub type IsColdAccess = bool;

#[derive(Codec, Default)]
pub struct BlockContext {
    pub chain_id: u64,
    pub coinbase: Address,
    pub timestamp: u64,
    pub number: u64,
    pub difficulty: U256,
    pub prev_randao: B256,
    pub gas_limit: u64,
    pub base_fee: U256,
}

impl From<&Env> for BlockContext {
    fn from(value: &Env) -> Self {
        Self {
            chain_id: value.cfg.chain_id,
            coinbase: value.block.coinbase,
            timestamp: value.block.timestamp.as_limbs()[0],
            number: value.block.number.as_limbs()[0],
            difficulty: value.block.difficulty,
            prev_randao: value.block.prevrandao.unwrap_or_default(),
            gas_limit: value.block.gas_limit.as_limbs()[0],
            base_fee: value.block.basefee,
        }
    }
}

#[derive(Codec, Default)]
pub struct TxContext {
    pub gas_limit: u64,
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_priority_fee: Option<U256>,
    pub origin: Address,
    pub data: Bytes,
    pub blob_hashes: Vec<B256>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub value: U256,
}

impl From<&Env> for TxContext {
    fn from(value: &Env) -> Self {
        Self {
            gas_limit: value.tx.gas_limit,
            nonce: value.tx.nonce.unwrap_or_default(),
            gas_price: value.tx.gas_price,
            gas_priority_fee: value.tx.gas_priority_fee,
            origin: value.tx.caller,
            data: value.tx.data.clone(),
            blob_hashes: value.tx.blob_hashes.clone(),
            max_fee_per_blob_gas: value.tx.max_fee_per_blob_gas,
            value: value.tx.value,
        }
    }
}

#[derive(Default)]
pub struct ContractContext {
    pub gas_limit: u64,
    pub address: Address,
    pub bytecode_address: Address,
    pub caller: Address,
    pub is_static: bool,
    pub value: U256,
    pub input: Bytes,
}

#[derive(Codec, Default)]
pub struct TransitStateInput {
    pub accounts: HashMap<Address, Account>,
    pub preimages: HashMap<B256, Bytes>,
    pub block: BlockContext,
    pub transaction: TxContext,
}

#[derive(Codec, Default)]
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
        // TODO(dmitry123): "RLP encoding here is temporary solution"
        use alloy_rlp::Encodable;
        let mut buffer = Vec::new();
        self.encode(&mut buffer);
        buffer.into()
    }
    pub fn from_bytes(buffer: Bytes) -> Self {
        // TODO(dmitry123): "RLP encoding here is temporary solution"
        use alloy_rlp::Decodable;
        let mut buffer_slice = buffer.as_ref();
        Self::decode(&mut buffer_slice).expect("failed to decode delegated execution")
    }
}

#[derive(Default)]
pub struct SovereignStateResult {
    pub accounts: Vec<Account>,
    pub storages: Vec<(Address, U256, U256)>,
    pub preimages: Vec<(B256, Bytes)>,
    pub logs: Vec<(Address, Bytes, Vec<B256>)>,
}

#[derive(Default)]
pub struct SharedStateResult {}

pub struct DestroyedAccountResult {
    pub had_value: bool,
    pub target_exists: bool,
    pub is_cold: bool,
    pub previously_destroyed: bool,
}

pub trait SovereignAPI {
    fn native_sdk(&self) -> &impl NativeAPI;

    fn block_context(&self) -> &BlockContext;
    fn tx_context(&self) -> &TxContext;
    fn contract_context(&self) -> Option<&ContractContext>;

    fn checkpoint(&self) -> JournalCheckpoint;
    fn commit(&mut self) -> SovereignStateResult;
    fn rollback(&mut self, checkpoint: JournalCheckpoint);

    fn write_account(&mut self, account: Account, status: AccountStatus);
    fn destroy_account(&mut self, address: &Address, target: &Address) -> DestroyedAccountResult;
    fn account(&self, address: &Address) -> (Account, IsColdAccess);
    fn account_committed(&self, address: &Address) -> (Account, IsColdAccess);

    fn write_preimage(&mut self, address: Address, hash: B256, preimage: Bytes);
    fn preimage(&self, hash: &B256) -> Option<Bytes>;
    fn preimage_size(&self, hash: &B256) -> u32;

    fn write_storage(&mut self, address: Address, slot: U256, value: U256) -> IsColdAccess;
    fn storage(&self, address: &Address, slot: &U256) -> (U256, IsColdAccess);
    fn committed_storage(&self, address: &Address, slot: &U256) -> (U256, IsColdAccess);

    fn write_transient_storage(&mut self, address: Address, index: U256, value: U256);
    fn transient_storage(&self, address: Address, index: U256) -> U256;

    fn write_log(&mut self, address: Address, data: Bytes, topics: &[B256]);

    fn context_call(
        &mut self,
        caller: &Address,
        address: &Address,
        value: &U256,
        fuel: &mut Fuel,
        input: &[u8],
        state: u32,
    ) -> (Bytes, ExitCode);

    fn precompile(
        &self,
        address: &Address,
        input: &Bytes,
        gas: u64,
    ) -> Option<(Bytes, ExitCode, u64, i64)>;
    fn is_precompile(&self, address: &Address) -> bool;
    fn transfer(
        &mut self,
        from: &mut Account,
        to: &mut Account,
        value: U256,
    ) -> Result<(), ExitCode>;
}

pub trait SharedAPI {
    fn native_sdk(&self) -> &impl NativeAPI;

    fn block_context(&self) -> &BlockContext;
    fn tx_context(&self) -> &TxContext;
    fn contract_context(&self) -> &ContractContext;

    fn commit(&mut self) -> SharedStateResult;

    fn account(&self, address: &Address) -> (Account, IsColdAccess);
    fn transfer(&mut self, from: &mut Account, to: &mut Account, amount: U256);
    fn write_storage(&mut self, slot: U256, value: U256);
    fn storage(&self, slot: &U256) -> U256;

    fn write_log(&mut self, data: Bytes, topics: &[B256]);

    fn call(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode);
    fn delegate(&mut self, address: Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode);

    fn exit(&mut self, exit_code: ExitCode) -> ! {
        self.commit();
        self.native_sdk().exit(exit_code.into_i32());
    }
}
