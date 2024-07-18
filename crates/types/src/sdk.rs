use crate::{
    account::{Account, AccountCheckpoint, AccountStatus},
    alloc_slice,
    Address,
    Bytes,
    ExitCode,
    Fuel,
    B256,
    F254,
    U256,
};
use alloc::vec::Vec;

/// A trait for reading context information.
pub trait ContextReader {
    fn block_chain_id(&self) -> u64;
    fn block_coinbase(&self) -> Address;
    fn block_timestamp(&self) -> u64;
    fn block_number(&self) -> u64;
    fn block_difficulty(&self) -> u64;
    fn block_prevrandao(&self) -> B256;
    fn block_gas_limit(&self) -> u64;
    fn block_base_fee(&self) -> U256;
    fn tx_gas_limit(&self) -> u64;
    fn tx_nonce(&self) -> u64;
    fn tx_gas_price(&self) -> U256;
    fn tx_caller(&self) -> Address;
    fn tx_access_list(&self) -> Vec<(Address, Vec<U256>)>;
    fn tx_gas_priority_fee(&self) -> Option<U256>;
    fn tx_blob_hashes(&self) -> Vec<B256>;
    fn tx_blob_hashes_size(&self) -> (u32, u32);
    fn tx_max_fee_per_blob_gas(&self) -> Option<U256>;
    fn contract_gas_limit(&self) -> u64;
    fn contract_address(&self) -> Address;
    fn contract_caller(&self) -> Address;
    fn contract_value(&self) -> U256;
    fn contract_is_static(&self) -> bool;
}

/// A trait for providing shared API functionality.
pub trait SharedAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(_data: &[u8]) -> B256 {
        unreachable!("sha256 is not supported yet")
    }
    fn poseidon(data: &[u8]) -> F254;
    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254;
    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65];

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

    fn account(&self, address: &Address) -> (Account, bool);
    fn preimage_size(&self, hash: &B256) -> u32;
    fn preimage_copy(&self, target: &mut [u8], hash: &B256);
    fn preimage(&self, hash: &B256) -> Bytes {
        let preimage_size = self.preimage_size(hash) as usize;
        let preimage = alloc_slice(preimage_size);
        self.preimage_copy(preimage, hash);
        Bytes::copy_from_slice(preimage)
    }
    fn log(&self, address: &Address, data: Bytes, topics: &[B256]);
    fn system_call(&self, address: &Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode);
    fn debug(&self, msg: &[u8]);
}

/// A trait for interacting with the sovereign blockchain.
///
/// This trait extends the `SharedAPI` trait and provides additional methods for
/// managing the blockchain state and executing transactions.
pub trait SovereignAPI: SharedAPI {
    fn checkpoint(&self) -> AccountCheckpoint;
    fn commit(&self);
    fn rollback(&self, checkpoint: AccountCheckpoint);
    fn write_account(&self, account: &Account, status: AccountStatus);
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]);
    fn context_call(
        &self,
        address: &Address,
        input: &[u8],
        context: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> (Bytes, ExitCode);
    fn storage(&self, address: &Address, slot: &U256, committed: bool) -> (U256, bool);
    fn write_storage(&self, address: &Address, slot: &U256, value: &U256) -> bool;
    fn write_log(&self, address: &Address, data: &Bytes, topics: &[B256]);

    fn precompile(
        &self,
        address: &Address,
        input: &Bytes,
        gas: u64,
    ) -> Option<(Bytes, ExitCode, u64, i64)>;
    fn is_precompile(&self, address: &Address) -> bool;
    fn transfer(&self, from: &mut Account, to: &mut Account, value: U256) -> Result<(), ExitCode>;
    fn self_destruct(&self, address: Address, target: Address) -> [bool; 4];
    fn block_hash(&self, number: U256) -> B256;
    fn write_transient_storage(&self, address: Address, index: U256, value: U256);
    fn transient_storage(&self, address: Address, index: U256) -> U256;
}
