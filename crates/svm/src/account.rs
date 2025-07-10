use crate::{
    // bpf_loader,
    clock::{Epoch, INITIAL_RENT_EPOCH},
    context::{IndexOfAccount, InstructionContext, TransactionContext},
    helpers::is_zeroed,
    solana_program::{loader_v4, sysvar::Sysvar},
    system_instruction::{
        MAX_PERMITTED_ACCOUNTS_DATA_ALLOCATIONS_PER_TRANSACTION,
        MAX_PERMITTED_DATA_LENGTH,
    },
};
use alloc::{rc::Rc, sync::Arc, vec, vec::Vec};
#[cfg(test)]
use core::fmt::Debug;
#[cfg(test)]
use core::fmt::Formatter;
use core::{
    cell::{Ref, RefCell, RefMut},
    mem::MaybeUninit,
    ptr,
};
use serde::{Deserialize, Serialize};
use solana_account_info::MAX_PERMITTED_DATA_INCREASE;
use solana_bincode::{deserialize, serialize, serialize_into, serialized_size};
use solana_instruction::error::{InstructionError, LamportsError};
use solana_pubkey::Pubkey;

pub type InheritableAccountFields = (u64, Epoch);
pub const DUMMY_INHERITABLE_ACCOUNT_FIELDS: InheritableAccountFields = (1, INITIAL_RENT_EPOCH);
/// Replacement for the executable flag: An account being owned by one of these contains a program.
pub const PROGRAM_OWNERS: &[Pubkey] = &[loader_v4::id()];
pub fn is_executable_by_owner(pk: &Pubkey) -> bool {
    PROGRAM_OWNERS.contains(pk)
}
pub fn is_executable_by_account(account: &AccountSharedData) -> bool {
    is_executable_by_owner(account.owner())
}

fn shared_deserialize_data<T: serde::de::DeserializeOwned, U: ReadableAccount>(
    account: &U,
) -> Result<T, bincode::error::DecodeError> {
    Ok(deserialize(account.data())?)
}

fn shared_serialize_data<T: serde::Serialize, U: WritableAccount>(
    account: &mut U,
    state: &T,
) -> Result<usize, bincode::error::EncodeError> {
    if serialized_size(state)? > account.data().len() {
        return Err(bincode::error::EncodeError::Other(
            "account data size limit",
        ));
    }
    serialize_into(state, account.data_as_mut_slice())
}

/// An Account with data that is stored on chain
#[repr(C)]
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Default /*, AbiExample*/)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// lamports in the account
    pub lamports: u64,
    /// data held in this account
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    /// the program that owns this account. If executable, the program that loads this account.
    pub owner: Pubkey,
    /// this account's data contains a loaded program (and is now read-only)
    pub executable: bool,
    /// the epoch at which this account will next owe rent
    pub rent_epoch: Epoch,
}

impl Account {
    pub fn new(lamports: u64, space: usize, owner: &Pubkey) -> Self {
        shared_new(lamports, space, owner)
    }
    pub fn new_ref(lamports: u64, space: usize, owner: &Pubkey) -> Rc<RefCell<Self>> {
        shared_new_ref(lamports, space, owner)
    }
    pub fn deserialize_data<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<T, bincode::error::DecodeError> {
        shared_deserialize_data(self)
    }
    pub fn serialize_data<T: serde::Serialize>(
        &mut self,
        state: &T,
    ) -> Result<usize, bincode::error::EncodeError> {
        shared_serialize_data(self, state)
    }
}

impl ReadableAccount for Account {
    fn lamports(&self) -> u64 {
        self.lamports
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
    fn owner(&self) -> &Pubkey {
        &self.owner
    }
    fn executable(&self) -> bool {
        self.executable
    }
    fn rent_epoch(&self) -> Epoch {
        self.rent_epoch
    }
}

impl WritableAccount for Account {
    fn set_lamports(&mut self, lamports: u64) {
        self.lamports = lamports;
    }
    fn data_as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
    fn set_owner(&mut self, owner: Pubkey) {
        self.owner = owner;
    }
    fn copy_into_owner_from_slice(&mut self, source: &[u8]) {
        self.owner.as_mut().copy_from_slice(source);
    }
    fn set_executable(&mut self, executable: bool) {
        self.executable = executable;
    }
    fn set_rent_epoch(&mut self, epoch: Epoch) {
        self.rent_epoch = epoch;
    }
    fn create(
        lamports: u64,
        data: Vec<u8>,
        owner: Pubkey,
        executable: bool,
        rent_epoch: Epoch,
    ) -> Self {
        Account {
            lamports,
            data,
            owner,
            executable,
            rent_epoch,
        }
    }
}

impl From<AccountSharedData> for Account {
    fn from(mut other: AccountSharedData) -> Self {
        let account_data = Arc::make_mut(&mut other.data);
        Self {
            lamports: other.lamports,
            data: core::mem::take(account_data),
            owner: other.owner,
            executable: other.executable,
            rent_epoch: other.rent_epoch,
        }
    }
}

impl From<Account> for AccountSharedData {
    fn from(other: Account) -> Self {
        Self {
            lamports: other.lamports,
            data: Arc::new(other.data),
            owner: other.owner,
            executable: other.executable,
            rent_epoch: other.rent_epoch,
        }
    }
}

pub trait ReadableAccount: Sized {
    fn lamports(&self) -> u64;
    fn data(&self) -> &[u8];
    fn owner(&self) -> &Pubkey;
    fn executable(&self) -> bool;
    fn rent_epoch(&self) -> Epoch;
    fn to_account_shared_data(&self) -> AccountSharedData {
        AccountSharedData::create(
            self.lamports(),
            self.data().to_vec(),
            *self.owner(),
            self.executable(),
            self.rent_epoch(),
        )
    }
}

pub trait WritableAccount: ReadableAccount {
    fn set_lamports(&mut self, lamports: u64);
    fn checked_add_lamports(&mut self, lamports: u64) -> Result<(), LamportsError> {
        self.set_lamports(
            self.lamports()
                .checked_add(lamports)
                .ok_or(LamportsError::ArithmeticOverflow)?,
        );
        Ok(())
    }
    fn checked_sub_lamports(&mut self, lamports: u64) -> Result<(), LamportsError> {
        self.set_lamports(
            self.lamports()
                .checked_sub(lamports)
                .ok_or(LamportsError::ArithmeticUnderflow)?,
        );
        Ok(())
    }
    fn saturating_add_lamports(&mut self, lamports: u64) {
        self.set_lamports(self.lamports().saturating_add(lamports))
    }
    fn saturating_sub_lamports(&mut self, lamports: u64) {
        self.set_lamports(self.lamports().saturating_sub(lamports))
    }
    fn data_as_mut_slice(&mut self) -> &mut [u8];
    fn set_owner(&mut self, owner: Pubkey);
    fn copy_into_owner_from_slice(&mut self, source: &[u8]);
    fn set_executable(&mut self, executable: bool);
    fn set_rent_epoch(&mut self, epoch: Epoch);
    fn create(
        lamports: u64,
        data: Vec<u8>,
        owner: Pubkey,
        executable: bool,
        rent_epoch: Epoch,
    ) -> Self;
}

fn shared_new<T: WritableAccount>(lamports: u64, space: usize, owner: &Pubkey) -> T {
    T::create(
        lamports,
        vec![0u8; space],
        *owner,
        bool::default(),
        Epoch::default(),
    )
}

fn shared_new_ref<T: WritableAccount>(
    lamports: u64,
    space: usize,
    owner: &Pubkey,
) -> Rc<RefCell<T>> {
    Rc::new(RefCell::new(shared_new::<T>(lamports, space, owner)))
}

fn shared_new_data<T: serde::Serialize, U: WritableAccount>(
    lamports: u64,
    state: &T,
    owner: &Pubkey,
) -> Result<U, bincode::error::EncodeError> {
    let data = serialize(state)?;
    Ok(U::create(
        lamports,
        data,
        *owner,
        bool::default(),
        Epoch::default(),
    ))
}

fn shared_new_data_with_space<T: serde::Serialize, U: WritableAccount>(
    lamports: u64,
    state: &T,
    space: usize,
    owner: &Pubkey,
) -> Result<U, bincode::error::EncodeError> {
    let mut account = shared_new::<U>(lamports, space, owner);

    shared_serialize_data(&mut account, state)?;

    Ok(account)
}

/// An Account with data that is stored on chain
/// This will be the in-memory representation of the 'Account' struct data.
/// The existing 'Account' structure cannot easily change due to downstream projects.
#[derive(PartialEq, Eq, Clone, Default, Serialize, Deserialize)]
pub struct AccountSharedData {
    /// lamports in the account
    lamports: u64,
    /// data held in this account
    data: Arc<Vec<u8>>,
    /// the program that owns this account. If executable, the program that loads this account.
    owner: Pubkey,
    /// this account's data contains a loaded program (and is now read-only)
    executable: bool,
    /// the epoch at which this account will next owe rent
    rent_epoch: Epoch,
}

#[cfg(test)]
impl Debug for AccountSharedData {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AccountSharedData")
            .field("lamports", &self.lamports)
            .field("owner", &self.owner)
            .field("executable", &self.executable)
            .field("rent_epoch", &self.rent_epoch)
            .finish()
    }
}

impl AccountSharedData {
    pub fn new(lamports: u64, space: usize, owner: &Pubkey) -> Self {
        shared_new(lamports, space, owner)
    }
    pub fn new_data<T: serde::Serialize>(
        lamports: u64,
        state: &T,
        owner: &Pubkey,
    ) -> Result<Self, bincode::error::EncodeError> {
        shared_new_data(lamports, state, owner)
    }

    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.data) > 1
    }

    pub fn reserve(&mut self, additional: usize) {
        self.data_mut().reserve(additional)
    }

    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    fn data_mut(&mut self) -> &mut Vec<u8> {
        Arc::make_mut(&mut self.data)
    }

    pub fn resize(&mut self, new_len: usize, value: u8) {
        self.data_mut().resize(new_len, value)
    }

    pub fn extend_from_slice(&mut self, data: &[u8]) {
        self.data_mut().extend_from_slice(data)
    }

    pub fn set_data_from_slice(&mut self, new_data: &[u8]) {
        // If the buffer isn't shared, we're going to memcpy in place.
        let Some(data) = Arc::get_mut(&mut self.data) else {
            // If the buffer is shared, the cheapest thing to do is to clone the
            // incoming slice and replace the buffer.
            return self.set_data(new_data.to_vec());
        };

        let new_len = new_data.len();

        // Reserve additional capacity if needed. Here we make the assumption
        // that growing the current buffer is cheaper than doing a whole new
        // allocation to make `new_data` owned.
        //
        // This assumption holds true during CPI, especially when the account
        // size doesn't change but the account is only changed in place. And
        // it's also true when the account is grown by a small margin (the
        // realloc limit is quite low), in which case the allocator can just
        // update the allocation metadata without moving.
        //
        // Shrinking and copying in place is always faster than making
        // `new_data` owned, since shrinking boils down to updating the Vec's length.

        data.reserve(new_len.saturating_sub(data.len()));

        // Safety:
        // We just reserved enough capacity. We set data::len to 0 to avoid
        // possible UB on panic (dropping uninitialized elements), do the copy,
        // finally set the new length once everything is initialized.
        #[allow(clippy::uninit_vec)]
        // this is a false positive, the lint doesn't currently special case set_len(0)
        unsafe {
            data.set_len(0);
            ptr::copy_nonoverlapping(new_data.as_ptr(), data.as_mut_ptr(), new_len);
            data.set_len(new_len);
        };
    }

    pub fn set_data(&mut self, data: Vec<u8>) {
        self.data = Arc::new(data);
    }

    pub fn spare_data_capacity_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        self.data_mut().spare_capacity_mut()
    }

    pub fn new_data_with_space<T: serde::Serialize>(
        lamports: u64,
        state: &T,
        space: usize,
        owner: &Pubkey,
    ) -> Result<Self, bincode::error::EncodeError> {
        shared_new_data_with_space(lamports, state, space, owner)
    }

    pub fn deserialize_data<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<T, bincode::error::DecodeError> {
        shared_deserialize_data(self)
    }
    pub fn serialize_data<T: serde::Serialize>(
        &mut self,
        state: &T,
    ) -> Result<usize, bincode::error::EncodeError> {
        shared_serialize_data(self, state)
    }
}

impl WritableAccount for AccountSharedData {
    fn set_lamports(&mut self, lamports: u64) {
        self.lamports = lamports;
    }
    fn data_as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data_mut()[..]
    }
    fn set_owner(&mut self, owner: Pubkey) {
        self.owner = owner;
    }
    fn copy_into_owner_from_slice(&mut self, source: &[u8]) {
        self.owner.as_mut().copy_from_slice(source);
    }
    fn set_executable(&mut self, executable: bool) {
        self.executable = executable;
    }
    fn set_rent_epoch(&mut self, epoch: Epoch) {
        self.rent_epoch = epoch;
    }
    fn create(
        lamports: u64,
        data: Vec<u8>,
        owner: Pubkey,
        executable: bool,
        rent_epoch: Epoch,
    ) -> Self {
        AccountSharedData {
            lamports,
            data: Arc::new(data),
            owner,
            executable,
            rent_epoch,
        }
    }
}

impl ReadableAccount for AccountSharedData {
    fn lamports(&self) -> u64 {
        self.lamports
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
    fn owner(&self) -> &Pubkey {
        &self.owner
    }
    fn executable(&self) -> bool {
        self.executable
    }
    fn rent_epoch(&self) -> Epoch {
        self.rent_epoch
    }
    fn to_account_shared_data(&self) -> AccountSharedData {
        // avoid data copy here
        self.clone()
    }
}

impl ReadableAccount for Ref<'_, AccountSharedData> {
    fn lamports(&self) -> u64 {
        self.lamports
    }
    fn data(&self) -> &[u8] {
        &self.data
    }
    fn owner(&self) -> &Pubkey {
        &self.owner
    }
    fn executable(&self) -> bool {
        self.executable
    }
    fn rent_epoch(&self) -> Epoch {
        self.rent_epoch
    }
    fn to_account_shared_data(&self) -> AccountSharedData {
        AccountSharedData {
            lamports: self.lamports(),
            data: Arc::clone(&self.data),
            owner: *self.owner(),
            executable: self.executable(),
            rent_epoch: self.rent_epoch(),
        }
    }
}

/// Shared account borrowed from the TransactionContext and an InstructionContext.
#[cfg_attr(test, derive(Debug))]
pub struct BorrowedAccount<'a> {
    pub(crate) transaction_context: &'a TransactionContext,
    pub(crate) instruction_context: &'a InstructionContext,
    pub(crate) index_in_transaction: IndexOfAccount,
    pub(crate) index_in_instruction: IndexOfAccount,
    pub(crate) account: RefMut<'a, AccountSharedData>,
}

impl<'a> BorrowedAccount<'a> {
    /// Returns the transaction context
    pub fn transaction_context(&self) -> &TransactionContext {
        self.transaction_context
    }

    /// Returns the index of this account (transaction wide)
    #[inline]
    pub fn get_index_in_transaction(&self) -> IndexOfAccount {
        self.index_in_transaction
    }

    /// Returns the public key of this account (transaction wide)
    #[inline]
    pub fn get_key(&self) -> &Pubkey {
        self.transaction_context
            .get_key_of_account_at_index(self.index_in_transaction)
            .unwrap()
    }

    /// Returns the owner of this account (transaction wide)
    #[inline]
    pub fn get_owner(&self) -> &Pubkey {
        self.account.owner()
    }

    /// Assignes the owner of this account (transaction wide)
    pub fn set_owner(&mut self, pubkey: &[u8]) -> Result<(), InstructionError> {
        // Only the owner can assign a new owner
        if !self.is_owned_by_current_program() {
            return Err(InstructionError::ModifiedProgramId);
        }
        // and only if the account is writable
        if !self.is_writable() {
            return Err(InstructionError::ModifiedProgramId);
        }
        // and only if the account is not executable
        if self.is_executable() {
            return Err(InstructionError::ModifiedProgramId);
        }
        // and only if the data is zero-initialized or empty
        if !is_zeroed(self.get_data()) {
            return Err(InstructionError::ModifiedProgramId);
        }
        // don't touch the account if the owner does not change
        if self.get_owner().to_bytes() == pubkey {
            return Ok(());
        }
        self.touch()?;
        self.account.copy_into_owner_from_slice(pubkey);
        Ok(())
    }

    /// Returns the number of lamports of this account (transaction wide)
    #[inline]
    pub fn get_lamports(&self) -> u64 {
        self.account.lamports()
    }

    /// Overwrites the number of lamports of this account (transaction wide)
    pub fn set_lamports(&mut self, lamports: u64) -> Result<(), InstructionError> {
        // An account not owned by the program cannot have its balance decrease
        if !self.is_owned_by_current_program() && lamports < self.get_lamports() {
            return Err(InstructionError::ExternalAccountLamportSpend);
        }
        // The balance of read-only may not change
        if !self.is_writable() {
            return Err(InstructionError::ReadonlyLamportChange);
        }
        // The balance of executable accounts may not change
        if self.is_executable() {
            return Err(InstructionError::ExecutableLamportChange);
        }
        // don't touch the account if the lamports do not change
        if self.get_lamports() == lamports {
            return Ok(());
        }
        self.touch()?;
        self.account.set_lamports(lamports);
        Ok(())
    }

    /// Adds lamports to this account (transaction wide)

    pub fn checked_add_lamports(&mut self, lamports: u64) -> Result<(), InstructionError> {
        self.set_lamports(
            self.get_lamports()
                .checked_add(lamports)
                .ok_or(InstructionError::ArithmeticOverflow)?,
        )
    }

    /// Subtracts lamports from this account (transaction wide)

    pub fn checked_sub_lamports(&mut self, lamports: u64) -> Result<(), InstructionError> {
        self.set_lamports(
            self.get_lamports()
                .checked_sub(lamports)
                .ok_or(InstructionError::ArithmeticOverflow)?,
        )
    }

    /// Returns a read-only slice of the account data (transaction wide)
    #[inline]
    pub fn get_data(&self) -> &[u8] {
        self.account.data()
    }

    /// Returns a writable slice of the account data (transaction wide)

    pub fn get_data_mut(&mut self) -> Result<&mut [u8], InstructionError> {
        self.can_data_be_changed()?;
        self.touch()?;
        self.make_data_mut();
        Ok(self.account.data_as_mut_slice())
    }

    /// Returns the spare capacity of the vector backing the account data.
    ///
    /// This method should only ever be used during CPI, where after a shrinking
    /// realloc we want to zero the spare capacity.

    pub fn spare_data_capacity_mut(&mut self) -> Result<&mut [MaybeUninit<u8>], InstructionError> {
        debug_assert!(!self.account.is_shared());
        Ok(self.account.spare_data_capacity_mut())
    }

    /// Overwrites the account data and size (transaction wide).
    ///
    /// You should always prefer set_data_from_slice(). Calling this method is
    /// currently safe but requires some special casing during CPI when direct
    /// account mapping is enabled.
    #[cfg(test)]
    pub fn set_data(&mut self, data: Vec<u8>) -> Result<(), InstructionError> {
        self.can_data_be_resized(data.len())?;
        self.can_data_be_changed()?;
        self.touch()?;

        self.update_accounts_resize_delta(data.len())?;
        self.account.set_data(data);
        Ok(())
    }

    /// Overwrites the account data and size (transaction wide).
    ///
    /// Call this when you have a slice of data you do not own and want to
    /// replace the account data with it.

    pub fn set_data_from_slice(&mut self, data: &[u8]) -> Result<(), InstructionError> {
        self.can_data_be_resized(data.len())?;
        self.can_data_be_changed()?;
        self.touch()?;
        self.update_accounts_resize_delta(data.len())?;
        // Calling make_data_mut() here guarantees that set_data_from_slice()
        // copies in places, extending the account capacity if necessary but
        // never reducing it. This is required as the account migh be directly
        // mapped into a MemoryRegion, and therefore reducing capacity would
        // leave a hole in the vm address space. After CPI or upon program
        // termination, the runtime will zero the extra capacity.
        self.make_data_mut();
        self.account.set_data_from_slice(data);

        Ok(())
    }

    /// Resizes the account data (transaction wide)
    ///
    /// Fills it with zeros at the end if is extended or truncates at the end otherwise.

    pub fn set_data_length(&mut self, new_length: usize) -> Result<(), InstructionError> {
        self.can_data_be_resized(new_length)?;
        self.can_data_be_changed()?;
        // don't touch the account if the length does not change
        if self.get_data().len() == new_length {
            return Ok(());
        }
        self.touch()?;
        self.update_accounts_resize_delta(new_length)?;
        self.account.resize(new_length, 0);
        Ok(())
    }

    /// Appends all elements in a slice to the account

    pub fn extend_from_slice(&mut self, data: &[u8]) -> Result<(), InstructionError> {
        let new_len = self.get_data().len().saturating_add(data.len());
        self.can_data_be_resized(new_len)?;
        self.can_data_be_changed()?;

        if data.is_empty() {
            return Ok(());
        }

        self.touch()?;
        self.update_accounts_resize_delta(new_len)?;
        // Even if extend_from_slice never reduces capacity, still realloc using
        // make_data_mut() if necessary so that we grow the account of the full
        // max realloc length in one go, avoiding smaller reallocations.
        self.make_data_mut();
        self.account.extend_from_slice(data);
        Ok(())
    }

    /// Reserves capacity for at least additional more elements to be inserted
    /// in the given account. Does nothing if capacity is already sufficient.
    pub fn reserve(&mut self, additional: usize) -> Result<(), InstructionError> {
        // Note that we don't need to call can_data_be_changed() here nor
        // touch() the account. reserve() only changes the capacity of the
        // memory that holds the account but it doesn't actually change content
        // nor length of the account.
        self.make_data_mut();
        self.account.reserve(additional);

        Ok(())
    }

    /// Returns the number of bytes the account can hold without reallocating.
    pub fn capacity(&self) -> usize {
        self.account.capacity()
    }

    /// Returns whether the underlying AccountSharedData is shared.
    ///
    /// The data is shared if the account has been loaded from the accounts database and has never
    /// been written to. Writing to an account unshares it.
    ///
    /// During account serialization, if an account is shared it'll get mapped as CoW, else it'll
    /// get mapped directly as writable.
    pub fn is_shared(&self) -> bool {
        self.account.is_shared()
    }

    fn make_data_mut(&mut self) {
        // if the account is still shared, it means this is the first time we're
        // about to write into it. Make the account mutable by copying it in a
        // buffer with MAX_PERMITTED_DATA_INCREASE capacity so that if the
        // transaction reallocs, we don't have to copy the whole account data a
        // second time to fullfill the realloc.
        //
        // NOTE: The account memory region CoW code in bpf_loader::create_vm() implements the same
        // logic and must be kept in sync.
        if self.account.is_shared() {
            self.account.reserve(MAX_PERMITTED_DATA_INCREASE);
        }
    }

    /// Deserializes the account data into a state
    pub fn get_state<T: serde::de::DeserializeOwned>(&self) -> Result<T, InstructionError> {
        self.account
            .deserialize_data()
            .map_err(|_| InstructionError::InvalidAccountData)
    }

    /// Serializes a state into the account data

    pub fn set_state<T: serde::Serialize>(&mut self, state: &T) -> Result<(), InstructionError> {
        let data = self.get_data_mut()?;
        let serialized_size = serialized_size(state).map_err(|_| InstructionError::GenericError)?;
        if serialized_size > data.len() {
            return Err(InstructionError::AccountDataTooSmall);
        }
        serialize_into(state, &mut *data).map_err(|_| InstructionError::GenericError)?;
        Ok(())
    }

    /// Returns whether this account is executable (transaction wide)
    #[inline]
    pub fn is_executable(&self) -> bool {
        self.account.executable()
    }

    /// Configures whether this account is executable (transaction wide)

    pub fn set_executable(&mut self, is_executable: bool) -> Result<(), InstructionError> {
        // Only the owner can set the executable flag
        if !self.is_owned_by_current_program() {
            return Err(InstructionError::ExecutableModified);
        }
        // and only if the account is writable
        if !self.is_writable() {
            return Err(InstructionError::ExecutableModified);
        }
        // one can not clear the executable flag
        if self.is_executable() && !is_executable {
            return Err(InstructionError::ExecutableModified);
        }
        // don't touch the account if the executable flag does not change
        if self.is_executable() == is_executable {
            return Ok(());
        }
        self.touch()?;
        self.account.set_executable(is_executable);
        Ok(())
    }

    /// Returns the rent epoch of this account (transaction wide)

    #[inline]
    pub fn get_rent_epoch(&self) -> u64 {
        self.account.rent_epoch()
    }

    /// Returns whether this account is a signer (instruction wide)
    pub fn is_signer(&self) -> bool {
        if self.index_in_instruction < self.instruction_context.get_number_of_program_accounts() {
            return false;
        }
        self.instruction_context
            .is_instruction_account_signer(
                self.index_in_instruction
                    .saturating_sub(self.instruction_context.get_number_of_program_accounts()),
            )
            .unwrap_or_default()
    }

    /// Returns whether this account is writable (instruction wide)
    pub fn is_writable(&self) -> bool {
        if self.index_in_instruction < self.instruction_context.get_number_of_program_accounts() {
            return false;
        }
        self.instruction_context
            .is_instruction_account_writable(
                self.index_in_instruction
                    .saturating_sub(self.instruction_context.get_number_of_program_accounts()),
            )
            .unwrap_or_default()
    }

    /// Returns true if the owner of this account is the current `InstructionContext`s last program (instruction wide)
    pub fn is_owned_by_current_program(&self) -> bool {
        let last_program_key_result = self
            .instruction_context
            .get_last_program_key(self.transaction_context);
        last_program_key_result
            .map(|key| key == self.get_owner())
            .unwrap_or_default()
    }

    /// Returns an error if the account data can not be mutated by the current program

    pub fn can_data_be_changed(&self) -> Result<(), InstructionError> {
        // Only non-executable accounts data can be changed
        if self.is_executable() {
            return Err(InstructionError::ExecutableDataModified);
        }
        // and only if the account is writable
        if !self.is_writable() {
            return Err(InstructionError::ReadonlyDataModified);
        }
        // and only if we are the owner
        if !self.is_owned_by_current_program() {
            return Err(InstructionError::ExternalAccountDataModified);
        }
        Ok(())
    }

    /// Returns an error if the account data can not be resized to the given length

    pub fn can_data_be_resized(&self, new_length: usize) -> Result<(), InstructionError> {
        let old_length = self.get_data().len();
        // Only the owner can change the length of the data
        if new_length != old_length && !self.is_owned_by_current_program() {
            return Err(InstructionError::AccountDataSizeChanged);
        }
        // The new length can not exceed the maximum permitted length
        if new_length > MAX_PERMITTED_DATA_LENGTH as usize {
            return Err(InstructionError::InvalidRealloc);
        }
        // The resize can not exceed the per-transaction maximum
        let length_delta = (new_length as i64).saturating_sub(old_length as i64);
        if self
            .transaction_context
            .accounts_resize_delta()?
            .saturating_add(length_delta)
            > MAX_PERMITTED_ACCOUNTS_DATA_ALLOCATIONS_PER_TRANSACTION
        {
            return Err(InstructionError::MaxAccountsDataAllocationsExceeded);
        }
        Ok(())
    }

    fn touch(&self) -> Result<(), InstructionError> {
        self.transaction_context
            .accounts()
            .touch(self.index_in_transaction)
    }

    fn update_accounts_resize_delta(&mut self, new_len: usize) -> Result<(), InstructionError> {
        let mut accounts_resize_delta = self
            .transaction_context
            .accounts_resize_delta
            .try_borrow_mut()
            .map_err(|_| InstructionError::GenericError)?;
        *accounts_resize_delta = accounts_resize_delta
            .saturating_add((new_len as i64).saturating_sub(self.get_data().len() as i64));
        Ok(())
    }
}

/// Serialize a `Sysvar` into an `Account`'s data.
pub fn to_account<S: Sysvar, T: WritableAccount>(sysvar: &S, account: &mut T) -> Option<()> {
    serialize_into(sysvar, account.data_as_mut_slice())
        .ok()
        .map(|_| ())
}
