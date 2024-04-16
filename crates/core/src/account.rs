use crate::account_types::{
    AccountCheckpoint, AccountFields, JZKT_ACCOUNT_BALANCE_FIELD, JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD, JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
};
use crate::helpers::{calc_create2_address, calc_create_address};
use crate::JZKT_ACCOUNT_FIELDS_COUNT;
use alloc::vec;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{
    Address, Bytes, ExitCode, B256, F254, KECCAK_EMPTY, NATIVE_TRANSFER_ADDRESS,
    NATIVE_TRANSFER_KECCAK, POSEIDON_EMPTY, U256,
};
use revm_primitives::AccountInfo;

#[derive(Debug, Clone)]
pub struct Account {
    pub address: Address,
    pub balance: U256,
    pub nonce: u64,
    pub source_code_size: u64,
    pub source_code_hash: B256,
    pub rwasm_code_size: u64,
    pub rwasm_code_hash: F254,
}

impl Into<AccountInfo> for Account {
    fn into(self) -> AccountInfo {
        AccountInfo {
            balance: self.balance,
            nonce: self.nonce,
            code_hash: self.source_code_hash,
            rwasm_code_hash: self.rwasm_code_hash,
            code: None,
            rwasm_code: None,
        }
    }
}

impl From<AccountInfo> for Account {
    fn from(value: AccountInfo) -> Self {
        Self {
            address: Address::ZERO,
            balance: value.balance,
            nonce: value.nonce,
            source_code_size: value
                .code
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or_default(),
            source_code_hash: value.code_hash,
            rwasm_code_size: value
                .rwasm_code
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or_default(),
            rwasm_code_hash: value.rwasm_code_hash,
        }
    }
}

impl Default for Account {
    fn default() -> Self {
        Self {
            address: Address::ZERO,
            rwasm_code_size: 0,
            source_code_size: 0,
            nonce: 0,
            balance: U256::ZERO,
            rwasm_code_hash: POSEIDON_EMPTY,
            source_code_hash: KECCAK_EMPTY,
        }
    }
}

impl Account {
    pub fn new(address: &Address) -> Self {
        Self {
            address: address.clone(),
            ..Default::default()
        }
    }

    pub fn new_from_fields(address: &Address, fields: &[Bytes32]) -> Self {
        let mut result = Self::new(address);
        assert_eq!(
            fields.len(),
            JZKT_ACCOUNT_FIELDS_COUNT as usize,
            "account fields len mismatch"
        );
        unsafe {
            result
                .balance
                .as_le_slice_mut()
                .copy_from_slice(&fields[JZKT_ACCOUNT_BALANCE_FIELD as usize]);
        }
        result.nonce = LittleEndian::read_u64(&fields[JZKT_ACCOUNT_NONCE_FIELD as usize]);
        result.source_code_size =
            LittleEndian::read_u64(&fields[JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD as usize]);
        result
            .source_code_hash
            .copy_from_slice(&fields[JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD as usize]);
        result.rwasm_code_size =
            LittleEndian::read_u64(&fields[JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD as usize]);
        result
            .rwasm_code_hash
            .copy_from_slice(&fields[JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD as usize]);
        result
    }

    pub fn new_from_jzkt(address: &Address) -> Self {
        let mut result = Self::new(address);
        let address_word = address.into_word();
        // code size and nonce
        let mut buffer32 = Bytes32::default();

        Account::jzkt_get_nonce(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.nonce = LittleEndian::read_u64(&buffer32);

        Account::jzkt_get_balance(address_word.as_ptr(), unsafe {
            result.balance.as_le_slice_mut().as_mut_ptr()
        });

        Account::jzkt_get_rwasm_bytecode_size(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.rwasm_code_size = LittleEndian::read_u64(&buffer32);

        Account::jzkt_get_rwasm_bytecode_hash(
            address_word.as_ptr(),
            result.rwasm_code_hash.as_mut_ptr(),
        );

        Account::jzkt_get_source_bytecode_size(address_word.as_ptr(), buffer32.as_mut_ptr());
        result.source_code_size = LittleEndian::read_u64(&buffer32);

        Account::jzkt_get_source_bytecode_hash(
            address_word.as_ptr(),
            result.source_code_hash.as_mut_ptr(),
        );

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
    pub fn jzkt_get_rwasm_bytecode_size(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_rwasm_bytecode_hash(address32_offset: *const u8, buffer32_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            buffer32_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_source_bytecode_size(address32_offset: *const u8, buffer32_le_offset: *mut u8) {
        LowLevelSDK::jzkt_get(
            address32_offset,
            JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
            buffer32_le_offset,
        );
    }

    #[inline]
    pub fn jzkt_get_source_bytecode_hash(address32_offset: *const u8, buffer32_offset: *mut u8) {
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
            &mut account_fields[JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD as usize][..],
            self.rwasm_code_size,
        );
        LittleEndian::write_u64(
            &mut account_fields[JZKT_ACCOUNT_NONCE_FIELD as usize][..],
            self.nonce,
        );
        account_fields[JZKT_ACCOUNT_BALANCE_FIELD as usize]
            .copy_from_slice(&self.balance.as_le_slice());

        account_fields[JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD as usize]
            .copy_from_slice(self.source_code_hash.as_slice());
        account_fields[JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD as usize]
            .copy_from_slice(self.rwasm_code_hash.as_slice());
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
            JZKT_ACCOUNT_COMPRESSION_FLAGS,
            account_fields.as_ptr(),
            32 * account_fields.len() as u32,
        );
    }

    pub fn inc_nonce(&mut self) -> Result<u64, ExitCode> {
        let prev_nonce = self.nonce;
        self.nonce += 1;
        if self.nonce == u64::MAX {
            return Err(ExitCode::NonceOverflow);
        }
        Ok(prev_nonce)
    }

    pub fn load_source_bytecode(&self) -> Bytes {
        let mut bytecode = vec![0u8; self.source_code_size as usize];
        LowLevelSDK::jzkt_preimage_copy(self.source_code_hash.as_ptr(), bytecode.as_mut_ptr());
        bytecode.into()
    }

    pub fn load_rwasm_bytecode(&self) -> Bytes {
        let mut bytecode = vec![0u8; self.rwasm_code_size as usize];
        LowLevelSDK::jzkt_preimage_copy(self.rwasm_code_hash.as_ptr(), bytecode.as_mut_ptr());
        bytecode.into()
    }

    pub fn update_bytecode(
        &mut self,
        source_bytecode: &Bytes,
        source_hash: Option<B256>,
        rwasm_bytecode: &Bytes,
        rwasm_hash: Option<F254>,
    ) {
        let address_word = self.address.into_word();
        // calc source code hash (we use keccak256 for backward compatibility)
        self.source_code_hash = source_hash.unwrap_or_else(|| {
            LowLevelSDK::crypto_keccak256(
                source_bytecode.as_ptr(),
                source_bytecode.len() as u32,
                self.source_code_hash.as_mut_ptr(),
            );
            self.source_code_hash
        });
        self.source_code_size = source_bytecode.len() as u64;
        // calc rwasm code hash (we use poseidon function for rWASM bytecode)
        self.rwasm_code_hash = rwasm_hash.unwrap_or_else(|| {
            LowLevelSDK::crypto_poseidon(
                rwasm_bytecode.as_ptr(),
                rwasm_bytecode.len() as u32,
                self.rwasm_code_hash.as_mut_ptr(),
            );
            self.rwasm_code_hash
        });
        self.rwasm_code_size = rwasm_bytecode.len() as u64;
        // write all changes to database
        self.write_to_jzkt();
        // make sure preimage of this hash is stored
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            source_bytecode.as_ptr(),
            source_bytecode.len() as u32,
        );
        assert!(r, "bytecode update failed");
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            rwasm_bytecode.as_ptr(),
            rwasm_bytecode.len() as u32,
        );
        assert!(r, "bytecode update failed");
    }

    #[deprecated(note = "use `update_bytecode` function to update both bytecodes")]
    pub fn update_source_bytecode(&mut self, bytecode: &Bytes) {
        let address_word = self.address.into_word();
        LowLevelSDK::crypto_keccak256(
            bytecode.as_ptr(),
            bytecode.len() as u32,
            self.source_code_hash.as_mut_ptr(),
        );
        self.source_code_size = bytecode.len() as u64;
        self.write_to_jzkt();
        // make sure preimage of this hash is stored
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            bytecode.as_ptr(),
            bytecode.len() as u32,
        );
        assert!(r, "account update_source_bytecode failed");
    }

    #[deprecated(note = "use `update_bytecode` function to update both bytecodes")]
    pub fn update_rwasm_bytecode(&mut self, bytecode: &Bytes, poseidon_hash: Option<F254>) {
        let address_word = self.address.into_word();
        self.rwasm_code_hash = poseidon_hash.unwrap_or_else(|| {
            let mut poseidon_hash = F254::ZERO;
            LowLevelSDK::crypto_poseidon(
                bytecode.as_ptr(),
                bytecode.len() as u32,
                poseidon_hash.as_mut_ptr(),
            );
            poseidon_hash
        });
        self.rwasm_code_size = bytecode.len() as u64;
        self.write_to_jzkt();
        // make sure preimage of this hash is stored
        let r = LowLevelSDK::jzkt_update_preimage(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            bytecode.as_ptr(),
            bytecode.len() as u32,
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
        LowLevelSDK::jzkt_rollback(checkpoint);
    }

    pub fn create_account(
        caller: &mut Account,
        amount: U256,
        salt_hash: Option<(B256, B256)>,
    ) -> Result<Account, ExitCode> {
        // check if caller have enough balance
        if caller.balance < amount {
            return Err(ExitCode::InsufficientBalance);
        }
        // try to increment nonce
        let old_nonce = caller.inc_nonce()?;
        // calc address
        let callee_address = if let Some((salt, hash)) = salt_hash {
            calc_create2_address(&caller.address, &salt, &hash)
        } else {
            calc_create_address(&caller.address, old_nonce)
        };
        let mut callee = Account::new_from_jzkt(&callee_address);
        // make sure there is no creation collision
        if callee.is_not_empty() {
            return Err(ExitCode::CreateCollision);
        }
        // change balance from caller and callee
        if let Err(exit_code) = Self::transfer(caller, &mut callee, amount) {
            return Err(exit_code);
        }
        // emit transfer log
        // Self::emit_transfer_log(&caller.address, &callee.address, &amount);
        // change nonce (we are always on spurious dragon)
        callee.nonce = 1;
        Ok(callee)
    }

    pub fn emit_transfer_log(from: &Address, to: &Address, amount: &U256) {
        let topics: [B256; 4] = [
            NATIVE_TRANSFER_KECCAK,
            from.into_word(),
            to.into_word(),
            B256::from(amount.to_be_bytes::<32>()),
        ];
        LowLevelSDK::jzkt_emit_log(
            NATIVE_TRANSFER_ADDRESS.as_ptr(),
            topics.as_ptr() as *const [u8; 32],
            4 * 32,
            core::ptr::null(),
            0,
        );
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
        Ok(())
    }

    #[inline(always)]
    pub fn is_not_empty(&self) -> bool {
        self.nonce != 0
            || self.source_code_hash != KECCAK_EMPTY
            || self.rwasm_code_hash != POSEIDON_EMPTY
    }
}
