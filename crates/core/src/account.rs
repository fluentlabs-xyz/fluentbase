use crate::account_types::{
    AccountCheckpoint,
    AccountFields,
    JZKT_ACCOUNT_AOT_CODE_HASH_FIELD,
    JZKT_ACCOUNT_AOT_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_COMPRESSION_FLAGS,
};
use alloc::vec;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, Bytes, ExitCode, B256, KECCAK_EMPTY, POSEIDON_EMPTY, U256};

#[derive(Debug, Clone)]
pub struct Account {
    pub address: Address,
    pub balance: U256,
    pub nonce: u64,
    pub source_code_size: u64,
    pub source_code_hash: B256,
    pub aot_code_size: u64,
    pub aot_code_hash: B256,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            address: Address::ZERO,
            aot_code_size: 0,
            source_code_size: 0,
            nonce: 0,
            balance: U256::ZERO,
            aot_code_hash: POSEIDON_EMPTY,
            source_code_hash: KECCAK_EMPTY,
        }
    }
}

impl Account {
    fn new(address: &Address) -> Self {
        Self {
            address: address.clone(),
            ..Default::default()
        }
    }

    pub fn new_from_jzkt(address: &Address) -> Self {
        let mut result = Self::new(address);
        let address_word = address.into_word();
        // code size and nonce
        let mut buffer32 = Bytes32::default();

        Account::jzkt_get_code_size(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.aot_code_size = LittleEndian::read_u64(&buffer32);

        Account::jzkt_get_nonce(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.nonce = LittleEndian::read_u64(&buffer32);

        Account::jzkt_get_balance(address_word.as_ptr(), unsafe {
            result.balance.as_le_slice_mut().as_mut_ptr()
        });

        Account::jzkt_get_source_code_hash(
            address_word.as_ptr(),
            result.source_code_hash.as_mut_ptr(),
        );

        Account::jzkt_get_code_hash(address_word.as_ptr(), result.aot_code_hash.as_mut_ptr());

        Account::jzkt_get_source_code_size(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.source_code_size = LittleEndian::read_u64(&buffer32);

        result
    }

    #[inline]
    pub fn jzkt_get_nonce(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_NONCE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_balance(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_BALANCE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_code_size(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_AOT_CODE_SIZE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_code_hash(address32_offset: *const u8, buffer32_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_AOT_CODE_HASH_FIELD,
            buffer32_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_source_code_size(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_source_code_hash(address32_offset: *const u8, buffer32_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            buffer32_offset,
        );
    }

    #[inline(always)]
    pub(crate) fn transfer_value(&mut self, to: &mut Self, value: &U256) -> bool {
        let from_balance = {
            let new_value = self.balance.checked_sub(*value);
            if new_value.is_none() {
                return false;
            }
            new_value.unwrap()
        };
        let to_balance = {
            let new_value = to.balance.checked_add(*value);
            if new_value.is_none() {
                return false;
            }
            new_value.unwrap()
        };
        self.balance = from_balance;
        to.balance = to_balance;
        true
    }

    pub fn get_fields(&self) -> AccountFields {
        let mut account_fields: AccountFields = Default::default();
        LittleEndian::write_u64(
            &mut account_fields[JZKT_ACCOUNT_AOT_CODE_SIZE_FIELD as usize][..],
            self.aot_code_size,
        );
        LittleEndian::write_u64(
            &mut account_fields[JZKT_ACCOUNT_NONCE_FIELD as usize][..],
            self.nonce,
        );
        account_fields[JZKT_ACCOUNT_BALANCE_FIELD as usize]
            .copy_from_slice(&self.balance.as_le_slice());

        account_fields[JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD as usize]
            .copy_from_slice(self.source_code_hash.as_slice());
        account_fields[JZKT_ACCOUNT_AOT_CODE_HASH_FIELD as usize]
            .copy_from_slice(self.aot_code_hash.as_slice());
        LittleEndian::write_u64(
            &mut account_fields[JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD as usize][..],
            self.source_code_size,
        );

        account_fields
    }

    pub fn write_to_jzkt(&self) {
        let account_fields = self.get_fields();

        LowLevelSDK::jzkt_update(
            self.address.into_word().as_ptr(),
            JZKT_COMPRESSION_FLAGS,
            account_fields.as_ptr(),
            32 * account_fields.len() as u32,
        );
    }

    pub fn inc_nonce(&mut self) -> u64 {
        let prev_nonce = self.nonce;
        self.nonce += 1;
        assert_ne!(self.nonce, u64::MAX);
        prev_nonce
    }

    #[inline]
    pub fn copy_bytecode(bytecode_hash32_offset: *const u8, output_offset: *mut u8) {
        LowLevelSDK::jzkt_preimage_copy(bytecode_hash32_offset, output_offset);
    }

    pub fn load_source_bytecode(&self) -> Bytes {
        let mut bytecode = vec![0u8; self.source_code_size as usize];
        Account::copy_bytecode(self.source_code_hash.as_ptr(), bytecode.as_mut_ptr());
        bytecode.into()
    }

    pub fn load_bytecode(&self) -> Bytes {
        let mut bytecode = vec![0u8; self.aot_code_size as usize];
        Account::copy_bytecode(self.aot_code_hash.as_ptr(), bytecode.as_mut_ptr());
        bytecode.into()
    }

    pub fn update_source_bytecode(&mut self, code: &Bytes) {
        let address_word = self.address.into_word();
        LowLevelSDK::crypto_keccak256(
            code.as_ptr(),
            code.len() as u32,
            self.source_code_hash.as_mut_ptr(),
        );
        self.source_code_size = code.len() as u64;
        self.write_to_jzkt();
        // make sure preimage of this hash is stored
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            code.as_ptr(),
            code.len() as u32,
        );
        assert!(r, "account update_source_bytecode failed");
    }

    pub fn update_rwasm_bytecode(&mut self, code: &Bytes) {
        let address_word = self.address.into_word();
        // refresh code hash
        LowLevelSDK::crypto_poseidon(
            code.as_ptr(),
            code.len() as u32,
            self.aot_code_hash.as_mut_ptr(),
        );
        self.aot_code_size = code.len() as u64;
        self.write_to_jzkt();
        // make sure preimage of this hash is stored
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_AOT_CODE_HASH_FIELD,
            code.as_ptr(),
            code.len() as u32,
        );
        assert!(r, "account update_bytecode failed");
    }

    pub fn checkpoint() -> AccountCheckpoint {
        LowLevelSDK::jzkt_checkpoint()
    }

    pub fn commit() -> B256 {
        let mut root = B256::ZERO;
        LowLevelSDK::jzkt_commit(root.as_mut_ptr());
        root
    }

    pub fn rollback(checkpoint: AccountCheckpoint) {
        LowLevelSDK::jzkt_rollback(checkpoint.0, checkpoint.1);
    }

    pub fn create_account_checkpoint(
        caller: &mut Account,
        callee: &mut Account,
        amount: U256,
    ) -> Result<AccountCheckpoint, ExitCode> {
        let checkpoint: AccountCheckpoint = Self::checkpoint();
        // make sure there is no creation collision
        if callee.aot_code_hash != POSEIDON_EMPTY || callee.nonce != 0 {
            LowLevelSDK::jzkt_rollback(checkpoint.0, checkpoint.1);
            return Err(ExitCode::CreateCollision);
        }
        // change balance from caller and callee
        caller.balance.checked_sub(amount).ok_or_else(|| {
            LowLevelSDK::jzkt_rollback(checkpoint.0, checkpoint.1);
            ExitCode::InsufficientBalance
        })?;
        callee.balance = callee.balance.checked_add(amount).ok_or_else(|| {
            LowLevelSDK::jzkt_rollback(checkpoint.0, checkpoint.1);
            ExitCode::OverflowPayment
        })?;
        // change nonce (we are always on spurious dragon)
        caller.nonce = 1;
        Ok(checkpoint)
    }

    pub fn sub_balance(&mut self, amount: U256) -> Result<(), ExitCode> {
        self.balance = self
            .balance
            .checked_sub(amount)
            .ok_or(ExitCode::InsufficientBalance)?;
        Ok(())
    }

    pub fn sub_balance_saturating(&mut self, amount: U256) {
        self.balance = self.balance.saturating_sub(amount);
    }

    pub fn add_balance(&mut self, amount: U256) -> Result<(), ExitCode> {
        self.balance = self
            .balance
            .checked_add(amount)
            .ok_or(ExitCode::OverflowPayment)?;
        Ok(())
    }

    pub fn add_balance_saturating(&mut self, amount: U256) {
        self.balance = self.balance.saturating_add(amount);
    }

    pub fn transfer(from: &mut Account, to: &mut Account, amount: U256) -> Result<(), ExitCode> {
        // update balances
        from.sub_balance(amount)?;
        to.add_balance(amount)?;
        // commit new balances into jzkt
        from.write_to_jzkt();
        to.write_to_jzkt();
        Ok(())
    }

    #[inline(always)]
    pub fn is_not_empty(&self) -> bool {
        self.nonce != 0
            || self.source_code_hash != KECCAK_EMPTY
            || self.aot_code_hash != POSEIDON_EMPTY
    }
}
