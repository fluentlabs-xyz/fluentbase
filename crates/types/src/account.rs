use crate::{
    utils::{calc_create2_address, calc_create_address},
    Address,
    Bytes,
    Bytes32,
    ExitCode,
    JournalCheckpoint,
    NativeAPI,
    SovereignAPI,
    B256,
    F254,
    KECCAK_EMPTY,
    NATIVE_TRANSFER_ADDRESS,
    NATIVE_TRANSFER_KECCAK,
    POSEIDON_EMPTY,
    U256,
};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_codec::Codec;
use revm_primitives::AccountInfo;

/// Number of fields
pub const JZKT_ACCOUNT_FIELDS_COUNT: u32 = 6;
pub const JZKT_STORAGE_FIELDS_COUNT: u32 = 1;

/// Account fields
pub const JZKT_ACCOUNT_BALANCE_FIELD: u32 = 0;
pub const JZKT_ACCOUNT_NONCE_FIELD: u32 = 1;
pub const JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD: u32 = 2;
pub const JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD: u32 = 3;
pub const JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD: u32 = 4;
pub const JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD: u32 = 5;

/// Compression flags for upper fields.
///
/// We compress the following fields:
/// - balance (0) because of balance overflow
/// - source code hash (3) because its keccak256
///
/// Mask is: 0b00001001
pub const JZKT_ACCOUNT_COMPRESSION_FLAGS: u32 =
    (1 << JZKT_ACCOUNT_BALANCE_FIELD) + (1 << JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD);
pub const JZKT_STORAGE_COMPRESSION_FLAGS: u32 = 0;

pub type AccountCheckpoint = u64;
pub type AccountFields = [Bytes32; JZKT_ACCOUNT_FIELDS_COUNT as usize];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccountStatus {
    NewlyCreated,
    Modified,
    SelfDestroyed,
    Transfer,
}

#[derive(Codec, Debug, Clone)]
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
        // yes, Ok is right error for nonce overflow
        self.nonce = self.nonce.checked_add(1).ok_or(ExitCode::Ok)?;
        Ok(prev_nonce)
    }

    pub fn update_bytecode<SDK: SovereignAPI>(
        &mut self,
        sdk: &mut SDK,
        source_bytecode: Bytes,
        source_hash: Option<B256>,
        rwasm_bytecode: Bytes,
        rwasm_hash: Option<F254>,
    ) {
        // calc source code hash (we use keccak256 for backward compatibility)
        self.source_code_hash =
            source_hash.unwrap_or_else(|| sdk.native_sdk().keccak256(source_bytecode.as_ref()));
        self.source_code_size = source_bytecode.len() as u64;
        // calc rwasm code hash (we use poseidon function for rWASM bytecode)
        self.rwasm_code_hash =
            rwasm_hash.unwrap_or_else(|| sdk.native_sdk().poseidon(rwasm_bytecode.as_ref()));
        self.rwasm_code_size = rwasm_bytecode.len() as u64;
        // write all changes to database
        sdk.write_account(self.clone(), AccountStatus::Modified);
        // make sure preimage of this hash is stored
        sdk.write_preimage(self.source_code_hash, source_bytecode);
        sdk.write_preimage(self.rwasm_code_hash, rwasm_bytecode);
    }

    pub fn create_account_checkpoint<SDK: SovereignAPI>(
        sdk: &mut SDK,
        caller: &mut Account,
        amount: U256,
        salt_hash: Option<(U256, B256)>,
    ) -> Result<(Account, JournalCheckpoint), ExitCode> {
        // check if caller has enough balances
        if caller.balance < amount {
            return Err(ExitCode::InsufficientBalance);
        }
        // try to increment nonce
        let old_nonce = caller.inc_nonce()?;
        sdk.write_account(caller.clone(), AccountStatus::Modified);
        // calc address
        let callee_address = if let Some((salt, hash)) = salt_hash {
            calc_create2_address(sdk.native_sdk(), &caller.address, &salt, &hash)
        } else {
            calc_create_address(sdk.native_sdk(), &caller.address, old_nonce)
        };
        // load account before checkpoint to keep it warm even in case of revert
        let (mut callee, _) = sdk.account(&callee_address);
        // create new checkpoint (before a loading account)
        let checkpoint = sdk.checkpoint();
        // make sure there is no creation collision
        let is_empty = callee.is_empty_code_hash() && callee.is_zero_nonce();
        if !is_empty || sdk.is_precompile(&callee_address) {
            return Err(ExitCode::CreateCollision);
        }
        // change balance from caller and callee
        sdk.transfer(caller, &mut callee, amount)?;
        // tidy hack to make SELFDESTRUCT work for now
        // am.mark_account_created(callee_address);
        // println!("mark account created: {callee_address}");
        // emit transfer log (do we want to have native transfer events or native wrapper?)
        // Self::emit_transfer_log(am, &caller.address, &callee.address, &amount);
        // change nonce (we are always on spurious dragon)
        let mut callee = callee.clone();
        callee.nonce = 1;
        // write account changes
        sdk.write_account(caller.clone(), AccountStatus::Modified);
        sdk.write_account(callee.clone(), AccountStatus::NewlyCreated);
        // return callee and checkpoint
        Ok((callee, checkpoint))
    }

    pub fn emit_transfer_log<SDK: SovereignAPI>(
        sdk: &mut SDK,
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
        sdk.write_log(NATIVE_TRANSFER_ADDRESS, Bytes::new(), &topics);
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
    pub fn is_zero_nonce(&self) -> bool {
        self.nonce == 0
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
