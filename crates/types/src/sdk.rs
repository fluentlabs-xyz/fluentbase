use crate::{
    alloc_vec,
    Account,
    AccountStatus,
    Address,
    Bytes,
    ExitCode,
    JournalCheckpoint,
    B256,
    F254,
    U256,
};
use alloc::{vec, vec::Vec};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use fluentbase_codec::Codec;

pub trait ContextFreeNativeAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(data: &[u8]) -> B256;

    /// Computes a quick hash of the given data using the Keccak256 algorithm or another specified
    /// hashing method.
    ///
    /// The hashing result produced by this function is not standardized and can vary depending on
    /// the proving system used.
    ///
    /// # Parameters
    /// - `data`: A byte slice representing the input data to be hashed.
    ///
    /// # Returns
    /// - `B256`: A 256-bit hash of the input data.
    fn hash256(data: &[u8]) -> B256 {
        // TODO(dmitry): "use the best hashing function here for our proving system"
        Self::keccak256(data)
    }

    fn poseidon(data: &[u8]) -> F254;
    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254;
    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65];
    fn debug_log(message: &str);
}

/// A trait for providing shared API functionality.
pub trait NativeAPI: ContextFreeNativeAPI {
    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;
    fn write(&self, value: &[u8]);
    fn forward_output(&self, offset: u32, len: u32);
    fn exit(&self, exit_code: i32) -> !;
    fn output_size(&self) -> u32;
    fn read_output(&self, target: &mut [u8], offset: u32);
    fn state(&self) -> u32;
    fn fuel(&self) -> u64;
    fn charge_fuel(&self, value: u64) -> u64;
    fn exec(&self, code_hash: &F254, input: &[u8], gas_limit: u64, state: u32) -> (u64, i32);
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_used: u64,
    ) -> (u64, i32);

    fn preimage_size(&self, hash: &B256) -> u32;
    fn preimage_copy(&self, hash: &B256, target: &mut [u8]);

    fn input(&self) -> Bytes {
        let input_size = self.input_size();
        let mut buffer = alloc_vec(input_size as usize);
        self.read(&mut buffer, 0);
        buffer.into()
    }

    fn return_data(&self) -> Bytes {
        let output_size = self.output_size();
        let mut buffer = alloc_vec(output_size as usize);
        self.read_output(&mut buffer, 0);
        buffer.into()
    }
}

pub type IsColdAccess = bool;

#[derive(Codec, Default, Clone)]
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

impl From<&revm_primitives::Env> for BlockContext {
    fn from(value: &revm_primitives::Env) -> Self {
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

#[derive(Codec, Default, Clone)]
pub struct TxContext {
    pub gas_limit: u64,
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_priority_fee: Option<U256>,
    pub origin: Address,
    // pub blob_hashes: Vec<B256>,
    // pub max_fee_per_blob_gas: Option<U256>,
    pub value: U256,
}

impl From<&revm_primitives::Env> for TxContext {
    fn from(value: &revm_primitives::Env) -> Self {
        Self {
            gas_limit: value.tx.gas_limit,
            nonce: value.tx.nonce.unwrap_or_default(),
            gas_price: value.tx.gas_price,
            gas_priority_fee: value.tx.gas_priority_fee,
            origin: value.tx.caller,
            // data: value.tx.data.clone(),
            // blob_hashes: value.tx.blob_hashes.clone(),
            // max_fee_per_blob_gas: value.tx.max_fee_per_blob_gas,
            value: value.tx.value,
        }
    }
}

#[derive(Default, Codec, Clone, Debug)]
pub struct ContractContext {
    pub address: Address,
    pub bytecode_address: Address,
    pub caller: Address,
    pub is_static: bool,
    pub value: U256,
}

pub fn env_from_context(
    block_context: &BlockContext,
    tx_context: &TxContext,
) -> revm_primitives::Env {
    use revm_primitives::{AnalysisKind, BlockEnv, CfgEnv, Env, TransactTo, TxEnv};
    Env {
        cfg: {
            let mut cfg_env = CfgEnv::default();
            cfg_env.chain_id = block_context.chain_id;
            cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
            cfg_env
        },
        block: BlockEnv {
            number: U256::from(block_context.number),
            coinbase: block_context.coinbase,
            timestamp: U256::from(block_context.timestamp),
            gas_limit: U256::from(block_context.gas_limit),
            basefee: block_context.base_fee,
            difficulty: block_context.difficulty,
            prevrandao: Some(block_context.prev_randao),
            blob_excess_gas_and_price: None,
        },
        tx: TxEnv {
            caller: tx_context.origin,
            gas_limit: tx_context.gas_limit,
            gas_price: tx_context.gas_price,
            // we don't check this field, and we don't know what type of "transact"
            // we execute right now, so can safely skip the field
            transact_to: TransactTo::Call(Address::ZERO),
            value: tx_context.value,
            // we don't use this field, so there is no need to do redundant copy operation
            data: Bytes::default(),
            // we do nonce and chain id checks before executing transaction
            nonce: None,
            chain_id: None,
            // we check access lists in advance before executing a smart contract, it
            // doesn't affect gas price or something else, can skip
            access_list: Default::default(),
            gas_priority_fee: tx_context.gas_priority_fee,
            // TODO(dmitry123): "we don't support blobs yet, so 2 tests from e2e testing suite fail"
            blob_hashes: vec![],        // tx_context.blob_hashes.clone(),
            max_fee_per_blob_gas: None, // tx_context.max_fee_per_blob_gas,
            authorization_list: None,
            #[cfg(feature = "optimism")]
            optimism: Default::default(),
        },
    }
}

#[derive(Codec, Default)]
pub struct SharedContextInputV1 {
    pub block: BlockContext,
    pub tx: TxContext,
    pub contract: ContractContext,
}

#[derive(Default)]
pub struct SovereignStateResult {
    pub accounts: Vec<Account>,
    pub storages: Vec<(Address, U256, U256)>,
    pub preimages: Vec<(B256, Bytes)>,
    pub logs: Vec<(Address, Bytes, Vec<B256>)>,
}

pub struct CallPrecompileResult {
    pub output: Bytes,
    pub exit_code: ExitCode,
    pub gas_remaining: u64,
    pub gas_refund: i64,
}

pub struct DestroyedAccountResult {
    pub had_value: bool,
    pub target_exists: bool,
    pub is_cold: bool,
    pub previously_destroyed: bool,
}

#[derive(Clone, Default, Debug, RlpEncodable, RlpDecodable)]
pub struct SyscallInvocationParams {
    pub code_hash: B256,
    pub input: Bytes,
    pub fuel_limit: u64,
    pub state: u32,
}

impl SyscallInvocationParams {
    pub fn to_vec(&self) -> Vec<u8> {
        // TODO(dmitry123): "replace RLP encoding/decoding with better serializer"
        use alloy_rlp::Encodable;
        let mut result = Vec::with_capacity(32 + 20 + 8 + 4 + self.input.len());
        self.encode(&mut result);
        result
    }

    pub fn from_slice(mut buffer: &[u8]) -> Option<Self> {
        use alloy_rlp::Decodable;
        Self::decode(&mut buffer).ok()
    }
}

pub trait SovereignAPI: ContextFreeNativeAPI {
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
    fn preimage(&self, address: &Address, hash: &B256) -> Option<Bytes>;
    fn preimage_size(&self, address: &Address, hash: &B256) -> u32;

    fn write_storage(&mut self, address: Address, slot: U256, value: U256) -> IsColdAccess;
    fn storage(&self, address: &Address, slot: &U256) -> (U256, IsColdAccess);
    fn committed_storage(&self, address: &Address, slot: &U256) -> (U256, IsColdAccess);

    fn write_transient_storage(&mut self, address: Address, index: U256, value: U256);
    fn transient_storage(&self, address: &Address, index: &U256) -> U256;

    fn write_log(&mut self, address: Address, data: Bytes, topics: Vec<B256>);

    fn precompile(
        &self,
        address: &Address,
        input: &Bytes,
        gas: u64,
    ) -> Option<CallPrecompileResult>;
    fn is_precompile(&self, address: &Address) -> bool;

    fn transfer(
        &mut self,
        from: &mut Account,
        to: &mut Account,
        value: U256,
    ) -> Result<(), ExitCode>;
}

pub trait SharedAPI: ContextFreeNativeAPI {
    fn block_context(&self) -> &BlockContext;
    fn tx_context(&self) -> &TxContext;
    fn contract_context(&self) -> &ContractContext;

    fn write_storage(&mut self, slot: U256, value: U256);
    fn storage(&self, slot: &U256) -> U256;
    fn write_transient_storage(&mut self, slot: U256, value: U256);
    fn transient_storage(&self, slot: &U256) -> U256;
    fn ext_storage(&self, address: &Address, slot: &U256) -> U256;

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;

    fn input(&self) -> Bytes {
        let input_size = self.input_size();
        let mut buffer = alloc_vec(input_size as usize);
        self.read(&mut buffer, 0);
        buffer.into()
    }

    fn charge_fuel(&self, value: u64);
    fn fuel(&self) -> u64;

    fn write(&mut self, output: &[u8]);
    fn exit(&self, exit_code: i32) -> !;

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]);
    fn preimage_size(&self, hash: &B256) -> u32;

    fn preimage(&self, hash: &B256) -> Bytes {
        let preimage_size = self.preimage_size(hash);
        let mut buffer = alloc_vec(preimage_size as usize);
        self.preimage_copy(hash, &mut buffer);
        buffer.into()
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]);

    fn balance(&self, address: &Address) -> U256;
    fn write_preimage(&mut self, preimage: Bytes) -> B256;
    fn create(
        &mut self,
        fuel_limit: u64,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> Result<Address, i32>;
    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: u64,
    ) -> (Bytes, i32);
    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: u64,
    ) -> (Bytes, i32);
    fn delegate_call(&mut self, address: Address, input: &[u8], fuel_limit: u64) -> (Bytes, i32);
    fn static_call(&mut self, address: Address, input: &[u8], fuel_limit: u64) -> (Bytes, i32);
    fn destroy_account(&mut self, address: Address);

    fn last_fuel_consumed(&self) -> u64 {
        0
    }
}
