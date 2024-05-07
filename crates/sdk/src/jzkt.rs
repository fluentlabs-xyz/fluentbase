use crate::utils::calc_storage_key;
use crate::{
    Account, AccountCheckpoint, AccountManager, LowLevelAPI, LowLevelSDK,
    JZKT_ACCOUNT_BALANCE_FIELD, JZKT_ACCOUNT_COMPRESSION_FLAGS, JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD, JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS,
};
use alloc::vec;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{Address, Bytes, Bytes32, B256, U256};

#[derive(Default)]
pub struct JzktAccountManager;

impl AccountManager for JzktAccountManager {
    #[inline(always)]
    fn checkpoint(&self) -> AccountCheckpoint {
        LowLevelSDK::jzkt_checkpoint()
    }

    #[inline(always)]
    fn commit(&self) {
        let mut root32: [u8; 32] = [0u8; 32];
        LowLevelSDK::jzkt_commit(root32.as_mut_ptr());
    }

    #[inline(always)]
    fn rollback(&self, account_checkpoint: AccountCheckpoint) {
        LowLevelSDK::jzkt_rollback(account_checkpoint);
    }

    #[inline(always)]
    fn account(&self, address: Address) -> (Account, bool) {
        let mut result = Account::new(address);
        let address_word = address.into_word();
        // code size and nonce
        let mut buffer32 = Bytes32::default();
        LowLevelSDK::jzkt_get(
            address_word.as_ptr(),
            JZKT_ACCOUNT_NONCE_FIELD,
            buffer32.as_mut_ptr(),
        );
        result.nonce = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::jzkt_get(address_word.as_ptr(), JZKT_ACCOUNT_BALANCE_FIELD, unsafe {
            result.balance.as_le_slice_mut().as_mut_ptr()
        });
        LowLevelSDK::jzkt_get(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
            buffer32.as_mut_ptr(),
        );
        result.rwasm_code_size = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::jzkt_get(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            result.rwasm_code_hash.as_mut_ptr(),
        );
        LowLevelSDK::jzkt_get(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
            buffer32.as_mut_ptr(),
        );
        result.source_code_size = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::jzkt_get(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            result.source_code_hash.as_mut_ptr(),
        );
        (result, true)
    }

    #[inline(always)]
    fn write_account(&self, account: &Account) {
        let account_address = account.address.into_word();
        let account_fields = account.get_fields();
        LowLevelSDK::jzkt_update(
            account_address.as_ptr(),
            JZKT_ACCOUNT_COMPRESSION_FLAGS,
            account_fields.as_ptr(),
            32 * account_fields.len() as u32,
        );
    }

    #[inline(always)]
    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        LowLevelSDK::jzkt_preimage_size(hash.as_ptr())
    }

    #[inline(always)]
    fn preimage(&self, hash: &[u8; 32]) -> Bytes {
        let preimage_size = LowLevelSDK::jzkt_preimage_size(hash.as_ptr()) as usize;
        let mut preimage = vec![0u8; preimage_size];
        LowLevelSDK::jzkt_preimage_copy(hash.as_ptr(), preimage.as_mut_ptr());
        preimage.into()
    }

    #[inline(always)]
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) {
        LowLevelSDK::jzkt_update_preimage(
            key.as_ptr(),
            field,
            preimage.as_ptr(),
            preimage.len() as u32,
        );
    }

    #[inline(always)]
    fn storage(&self, address: Address, slot: U256) -> (U256, bool) {
        let mut value = U256::ZERO;
        let storage_key = calc_storage_key(address, slot.as_le_slice().as_ptr());
        let is_cold = LowLevelSDK::jzkt_get(storage_key.as_ptr(), 0, unsafe {
            value.as_le_slice_mut().as_mut_ptr()
        });
        (value, is_cold)
    }

    #[inline(always)]
    fn write_storage(&self, address: Address, slot: U256, value: U256) -> bool {
        let storage_key = calc_storage_key(address, slot.as_le_slice().as_ptr());
        LowLevelSDK::jzkt_update(
            storage_key.as_ptr(),
            JZKT_STORAGE_COMPRESSION_FLAGS,
            value.as_le_slice().as_ptr() as *const [u8; 32],
            32,
        );
        true
    }

    fn log(&self, address: Address, data: Bytes, topics: &[B256]) {
        LowLevelSDK::jzkt_emit_log(
            address.as_ptr(),
            // we can do such cast because B256 has transparent repr
            topics.as_ptr() as *const [u8; 32],
            topics.len() as u32 * 32,
            data.as_ptr(),
            data.len() as u32,
        );
    }

    fn exec_hash(
        &self,
        hash32_offset: *const u8,
        input: &[u8],
        fuel_offset: *mut u32,
        state: u32,
    ) -> (Bytes, i32) {
        let exit_code = LowLevelSDK::sys_exec_hash(
            hash32_offset,
            input.as_ptr(),
            input.len() as u32,
            core::ptr::null_mut(),
            0,
            fuel_offset,
            state,
        );
        let out_size = LowLevelSDK::sys_output_size();
        let mut output_buffer = vec![0u8; out_size as usize];
        LowLevelSDK::sys_read_output(output_buffer.as_mut_ptr(), 0, out_size);
        (output_buffer.into(), exit_code)
    }
}
