use crate::{
    utils::{calc_create2_address, calc_create_address},
    EvmCallMethodOutput,
    LowLevelSDK,
    SharedAPI,
};
use alloc::vec;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{
    Address,
    Bytes,
    Bytes32,
    ExitCode,
    B256,
    F254,
    KECCAK_EMPTY,
    NATIVE_TRANSFER_ADDRESS,
    NATIVE_TRANSFER_KECCAK,
    POSEIDON_EMPTY,
    U256,
};
use revm_primitives::AccountInfo;

/// Number of fields
pub const JZKT_ACCOUNT_FIELDS_COUNT: u32 = 6;
pub const JZKT_STORAGE_FIELDS_COUNT: u32 = 1;

pub const JZKT_ACCOUNT_BALANCE_FIELD: u32 = 0;
pub const JZKT_ACCOUNT_NONCE_FIELD: u32 = 1;
pub const JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD: u32 = 2;
pub const JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD: u32 = 3;
pub const JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD: u32 = 4;
pub const JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD: u32 = 5;

/// Compression flags for upper fields.
///
/// We compress following fields:
/// - balance (0) because of balance overflow
/// - source code hash (3) because its keccak256
///
/// Mask is: 0b00001001
pub const JZKT_ACCOUNT_COMPRESSION_FLAGS: u32 =
    (1 << JZKT_ACCOUNT_BALANCE_FIELD) + (1 << JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD);
pub const JZKT_STORAGE_COMPRESSION_FLAGS: u32 = 0;

pub type AccountCheckpoint = u64;
pub type AccountFields = [Bytes32; JZKT_ACCOUNT_FIELDS_COUNT as usize];

pub trait AccountManager {
    fn checkpoint(&self) -> AccountCheckpoint;
    fn commit(&self);
    fn rollback(&self, checkpoint: AccountCheckpoint);
    fn account(&self, address: Address) -> (Account, bool);
    fn write_account(&self, account: &Account);
    fn preimage_size(&self, hash: &[u8; 32]) -> u32;
    fn preimage(&self, hash: &[u8; 32]) -> Bytes;
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]);
    fn storage(&self, address: Address, slot: U256, committed: bool) -> (U256, bool);
    fn write_storage(&self, address: Address, slot: U256, value: U256) -> bool;
    fn log(&self, address: Address, data: Bytes, topics: &[B256]);
    fn exec_hash(
        &self,
        hash32_offset: *const u8,
        context: &[u8],
        input: &[u8],
        fuel_offset: *mut u32,
        state: u32,
    ) -> (Bytes, i32);
    fn inc_nonce(&self, account: &mut Account) -> Option<u64>;
    fn transfer(&self, from: &mut Account, to: &mut Account, value: U256) -> Result<(), ExitCode>;
    fn precompile(&self, address: &Address, input: &Bytes, gas: u64)
        -> Option<EvmCallMethodOutput>;
    fn is_precompile(&self, address: &Address) -> bool;
    fn self_destruct(&self, address: Address, target: Address) -> [bool; 4];
    fn block_hash(&self, number: U256) -> B256;
    fn write_transient_storage(&self, address: Address, index: U256, value: U256);
    fn transient_storage(&self, address: Address, index: U256) -> U256;
    fn mark_account_created(&self, address: Address);
}

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
    pub fn new(address: Address) -> Self {
        Self {
            address,
            ..Default::default()
        }
    }

    pub fn new_from_fields(address: Address, fields: &[Bytes32]) -> Self {
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

    pub fn inc_nonce(&mut self) -> Result<u64, ExitCode> {
        let prev_nonce = self.nonce;
        self.nonce = self.nonce.checked_add(1).ok_or(ExitCode::NonceOverflow)?;
        Ok(prev_nonce)
    }

    pub fn update_bytecode<AM: AccountManager>(
        &mut self,
        am: &AM,
        source_bytecode: &Bytes,
        source_hash: Option<B256>,
        rwasm_bytecode: &Bytes,
        rwasm_hash: Option<F254>,
    ) {
        let address_word = self.address.into_word();
        // calc source code hash (we use keccak256 for backward compatibility)
        self.source_code_hash = source_hash.unwrap_or_else(|| {
            LowLevelSDK::keccak256(
                source_bytecode.as_ptr(),
                source_bytecode.len() as u32,
                self.source_code_hash.as_mut_ptr(),
            );
            self.source_code_hash
        });
        self.source_code_size = source_bytecode.len() as u64;
        // calc rwasm code hash (we use poseidon function for rWASM bytecode)
        self.rwasm_code_hash = rwasm_hash.unwrap_or_else(|| {
            LowLevelSDK::poseidon(
                rwasm_bytecode.as_ptr(),
                rwasm_bytecode.len() as u32,
                self.rwasm_code_hash.as_mut_ptr(),
            );
            self.rwasm_code_hash
        });
        self.rwasm_code_size = rwasm_bytecode.len() as u64;
        // write all changes to database
        am.write_account(self);
        // make sure preimage of this hash is stored
        am.update_preimage(
            &address_word,
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            source_bytecode.as_ref(),
        );
        am.update_preimage(
            &address_word,
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            rwasm_bytecode.as_ref(),
        );
    }

    pub fn create_account_checkpoint<AM: AccountManager>(
        am: &AM,
        caller: &mut Account,
        amount: U256,
        salt_hash: Option<(U256, B256)>,
    ) -> Result<(Account, AccountCheckpoint), ExitCode> {
        // check if caller have enough balance
        if caller.balance < amount {
            return Err(ExitCode::InsufficientBalance);
        }
        // try to increment nonce
        let old_nonce = am.inc_nonce(caller).ok_or(ExitCode::Ok)?;
        // calc address
        let callee_address = if let Some((salt, hash)) = salt_hash {
            calc_create2_address(&caller.address, &salt, &hash)
        } else {
            calc_create_address(&caller.address, old_nonce)
        };
        // load account before checkpoint to keep it warm even in case of revert
        let (mut callee, _) = am.account(callee_address);
        // create new checkpoint (before loading account)
        let checkpoint = am.checkpoint();
        // make sure there is no creation collision
        let is_empty = callee.is_empty_code_hash() && callee.nonce == 0;
        if !is_empty || am.is_precompile(&callee_address) {
            return Err(ExitCode::CreateCollision);
        }
        // tidy hack to make SELFDESTRUCT work for now
        // TODO stas: this creates incorrect behavior for revm-rwasm test:
        //  tests/GeneralStateTests/stCreateTest/CreateAddressWarmAfterFail.json
        am.mark_account_created(callee_address);
        // change balance from caller and callee
        if let Err(exit_code) = am.transfer(caller, &mut callee, amount) {
            return Err(exit_code);
        }
        // emit transfer log (do we want to have native transfer events or native wrapper?)
        // Self::emit_transfer_log(&caller.address, &callee.address, &amount);
        // change nonce (we are always on spurious dragon)
        am.inc_nonce(&mut callee);
        // write account changes
        am.write_account(&caller);
        am.write_account(&callee);
        // return callee and checkpoint
        Ok((callee, checkpoint))
    }

    pub fn emit_transfer_log<AM: AccountManager>(
        am: &AM,
        from: &Address,
        to: &Address,
        amount: &U256,
    ) {
        let topics: [B256; 4] = [
            NATIVE_TRANSFER_KECCAK,
            from.into_word(),
            to.into_word(),
            B256::from(amount.to_be_bytes::<32>()),
        ];
        am.log(Address::ZERO, Bytes::new(), &topics);
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
        let from_balance = from
            .balance
            .checked_sub(amount)
            .ok_or(ExitCode::InsufficientBalance)?;
        let to_balance = to
            .balance
            .checked_add(amount)
            .ok_or(ExitCode::OverflowPayment)?;
        from.balance = from_balance;
        to.balance = to_balance;
        Ok(())
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        let code_empty = self.is_empty_code_hash() || self.is_zero_code_hash();
        code_empty && self.balance == U256::ZERO && self.nonce == 0
    }

    #[inline(always)]
    pub fn is_empty_code_hash(&self) -> bool {
        self.source_code_hash == KECCAK_EMPTY && self.rwasm_code_hash == POSEIDON_EMPTY
    }

    #[inline(always)]
    pub fn is_zero_code_hash(&self) -> bool {
        self.source_code_hash == B256::ZERO && self.rwasm_code_hash == B256::ZERO
    }

    #[inline(always)]
    pub fn is_not_empty(&self) -> bool {
        !self.is_empty()
    }
}
