//! Extensions available to token mints and accounts

use crate::token_2022::extension::transfer_fee::{TransferFeeAmount, TransferFeeConfig};
use crate::token_2022::libraries::variable_len_pack::VariableLenPack;
use alloc::vec;
use alloc::vec::Vec;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use solana_program_error::ProgramError;
use solana_program_pack::IsInitialized;
use {
    crate::token_2022::spl_pod::{
        bytemuck::{pod_from_bytes, pod_from_bytes_mut, pod_get_packed_len},
        primitives::PodU16,
    },
    crate::{
        error::TokenError,
        token_2022::extension::{
            default_account_state::DefaultAccountState,
            immutable_owner::ImmutableOwner,
            mint_close_authority::MintCloseAuthority,
            non_transferable::{NonTransferable, NonTransferableAccount},
        },
        token_2022::pod::{PodAccount, PodMint},
        token_2022::state::{Account, Mint, Multisig, PackedSizeOf},
    },
    bytemuck::{Pod, Zeroable},
    core::{
        cmp::Ordering,
        convert::{TryFrom, TryInto},
        mem::size_of,
    },
    solana_program_pack::Pack,
};

// /// CPI Guard extension
// pub mod cpi_guard;
/// Default Account State extension
pub mod default_account_state;
// /// Group Member Pointer extension
// pub mod group_member_pointer;
// /// Group Pointer extension
// pub mod group_pointer;
/// Immutable Owner extension
pub mod immutable_owner;
// /// Interest-Bearing Mint extension
// pub mod interest_bearing_mint;
// /// Memo Transfer extension
// pub mod memo_transfer;
// /// Metadata Pointer extension
// pub mod metadata_pointer;
/// Mint Close Authority extension
pub mod mint_close_authority;
/// Non Transferable extension
pub mod non_transferable;
// /// Permanent Delegate extension
// pub mod permanent_delegate;
/// Utility to reallocate token accounts
pub mod reallocate;
// /// Token-group extension
// pub mod token_group;
// /// Token-metadata extension
// pub mod token_metadata;
// /// Transfer Fee extension
pub mod transfer_fee;
// /// Transfer Hook extension
// pub mod transfer_hook;
//
// /// Confidential mint-burn extension
// pub mod confidential_mint_burn;
//
/// Length in TLV structure
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(transparent)]
pub struct Length(PodU16);
impl From<Length> for usize {
    fn from(n: Length) -> Self {
        Self::from(u16::from(n.0))
    }
}
impl TryFrom<usize> for Length {
    type Error = ProgramError;
    fn try_from(n: usize) -> Result<Self, Self::Error> {
        u16::try_from(n)
            .map(|v| Self(PodU16::from(v)))
            .map_err(|_| ProgramError::AccountDataTooSmall)
    }
}

/// Helper function to get the current TlvIndices from the current spot
fn get_tlv_indices(type_start: usize) -> TlvIndices {
    let length_start = type_start.saturating_add(size_of::<ExtensionType>());
    let value_start = length_start.saturating_add(pod_get_packed_len::<Length>());
    TlvIndices {
        type_start,
        length_start,
        value_start,
    }
}

/// Helper function to tack on the size of an extension bytes if an account with
/// extensions is exactly the size of a multisig
const fn adjust_len_for_multisig(account_len: usize) -> usize {
    if account_len == Multisig::LEN {
        account_len.saturating_add(size_of::<ExtensionType>())
    } else {
        account_len
    }
}

/// Helper function to calculate exactly how many bytes a value will take up,
/// given the value's length
const fn add_type_and_length_to_len(value_len: usize) -> usize {
    value_len
        .saturating_add(size_of::<ExtensionType>())
        .saturating_add(pod_get_packed_len::<Length>())
}

/// Helper struct for returning the indices of the type, length, and value in
/// a TLV entry
#[derive(Debug)]
struct TlvIndices {
    pub type_start: usize,
    pub length_start: usize,
    pub value_start: usize,
}
fn get_extension_indices<V: Extension>(
    tlv_data: &[u8],
    init: bool,
) -> Result<TlvIndices, ProgramError> {
    let mut start_index = 0;
    let v_account_type = V::TYPE.get_account_type();
    while start_index < tlv_data.len() {
        let tlv_indices = get_tlv_indices(start_index);
        if tlv_data.len() < tlv_indices.value_start {
            return Err(ProgramError::InvalidAccountData);
        }
        let extension_type =
            ExtensionType::try_from(&tlv_data[tlv_indices.type_start..tlv_indices.length_start])?;
        let account_type = extension_type.get_account_type();
        if extension_type == V::TYPE {
            // found an instance of the extension that we're initializing, return!
            return Ok(tlv_indices);
        // got to an empty spot, init here, or error if we're searching, since
        // nothing is written after an Uninitialized spot
        } else if extension_type == ExtensionType::Uninitialized {
            if init {
                return Ok(tlv_indices);
            } else {
                return Err(TokenError::ExtensionNotFound.into());
            }
        } else if v_account_type != account_type {
            return Err(TokenError::ExtensionTypeMismatch.into());
        } else {
            let length = pod_from_bytes::<Length>(
                &tlv_data[tlv_indices.length_start..tlv_indices.value_start],
            )?;
            let value_end_index = tlv_indices.value_start.saturating_add(usize::from(*length));
            start_index = value_end_index;
        }
    }
    Err(ProgramError::InvalidAccountData)
}

/// Basic information about the TLV buffer, collected from iterating through all
/// entries
#[derive(Debug, PartialEq)]
struct TlvDataInfo {
    /// The extension types written in the TLV buffer
    extension_types: Vec<ExtensionType>,
    /// The total number bytes allocated for all TLV entries.
    ///
    /// Each TLV entry's allocated bytes comprises two bytes for the `type`, two
    /// bytes for the `length`, and `length` number of bytes for the `value`.
    used_len: usize,
}

/// Fetches basic information about the TLV buffer by iterating through all
/// TLV entries.
fn get_tlv_data_info(tlv_data: &[u8]) -> Result<TlvDataInfo, ProgramError> {
    let mut extension_types = vec![];
    let mut start_index = 0;
    while start_index < tlv_data.len() {
        let tlv_indices = get_tlv_indices(start_index);
        if tlv_data.len() < tlv_indices.length_start {
            // There aren't enough bytes to store the next type, which means we
            // got to the end. The last byte could be used during a realloc!
            return Ok(TlvDataInfo {
                extension_types,
                used_len: tlv_indices.type_start,
            });
        }
        let extension_type =
            ExtensionType::try_from(&tlv_data[tlv_indices.type_start..tlv_indices.length_start])?;
        if extension_type == ExtensionType::Uninitialized {
            return Ok(TlvDataInfo {
                extension_types,
                used_len: tlv_indices.type_start,
            });
        } else {
            if tlv_data.len() < tlv_indices.value_start {
                // not enough bytes to store the length, malformed
                return Err(ProgramError::InvalidAccountData);
            }
            extension_types.push(extension_type);
            let length = pod_from_bytes::<Length>(
                &tlv_data[tlv_indices.length_start..tlv_indices.value_start],
            )?;

            let value_end_index = tlv_indices.value_start.saturating_add(usize::from(*length));
            if value_end_index > tlv_data.len() {
                // value blows past the size of the slice, malformed
                return Err(ProgramError::InvalidAccountData);
            }
            start_index = value_end_index;
        }
    }
    Ok(TlvDataInfo {
        extension_types,
        used_len: start_index,
    })
}

fn get_first_extension_type(tlv_data: &[u8]) -> Result<Option<ExtensionType>, ProgramError> {
    if tlv_data.is_empty() {
        Ok(None)
    } else {
        let tlv_indices = get_tlv_indices(0);
        if tlv_data.len() <= tlv_indices.length_start {
            return Ok(None);
        }
        let extension_type =
            ExtensionType::try_from(&tlv_data[tlv_indices.type_start..tlv_indices.length_start])?;
        if extension_type == ExtensionType::Uninitialized {
            Ok(None)
        } else {
            Ok(Some(extension_type))
        }
    }
}

fn check_min_len_and_not_multisig(input: &[u8], minimum_len: usize) -> Result<(), ProgramError> {
    if
    /*input.len() == Multisig::LEN || */
    input.len() < minimum_len {
        Err(ProgramError::InvalidAccountData)
    } else {
        Ok(())
    }
}

fn check_account_type<S: BaseState>(account_type: AccountType) -> Result<(), ProgramError> {
    if account_type != S::ACCOUNT_TYPE {
        Err(ProgramError::InvalidAccountData)
    } else {
        Ok(())
    }
}

/// Any account with extensions must be at least `Account::LEN`.  Both mints and
/// accounts can have extensions
/// A mint with extensions that takes it past 165 could be indiscernible from an
/// Account with an extension, even if we add the account type. For example,
/// let's say we have:
///
/// Account: 165 bytes... + [2, 0, 3, 0, 100, ....]
///                          ^     ^       ^     ^
///                     acct type  extension length data...
///
/// Mint: 82 bytes... + 83 bytes of other extension data
///     + [2, 0, 3, 0, 100, ....]
///      (data in extension just happens to look like this)
///
/// With this approach, we only start writing the TLV data after Account::LEN,
/// which means we always know that the account type is going to be right after
/// that. We do a special case checking for a Multisig length, because those
/// aren't extensible under any circumstances.
const BASE_ACCOUNT_LENGTH: usize = Account::LEN;
/// Helper that tacks on the AccountType length, which gives the minimum for any
/// account with extensions
const BASE_ACCOUNT_AND_TYPE_LENGTH: usize = BASE_ACCOUNT_LENGTH + size_of::<AccountType>();

fn type_and_tlv_indices<S: BaseState>(
    rest_input: &[u8],
) -> Result<Option<(usize, usize)>, ProgramError> {
    if rest_input.is_empty() {
        Ok(None)
    } else {
        let account_type_index = BASE_ACCOUNT_LENGTH.saturating_sub(S::SIZE_OF);
        // check padding is all zeroes
        let tlv_start_index = account_type_index.saturating_add(size_of::<AccountType>());
        if rest_input.len() <= tlv_start_index {
            return Err(ProgramError::InvalidAccountData);
        }
        if rest_input[..account_type_index] != vec![0; account_type_index] {
            Err(ProgramError::InvalidAccountData)
        } else {
            Ok(Some((account_type_index, tlv_start_index)))
        }
    }
}

/// Checks a base buffer to verify if it is an Account without having to
/// completely deserialize it
fn is_initialized_account(input: &[u8]) -> Result<bool, ProgramError> {
    const ACCOUNT_INITIALIZED_INDEX: usize = 108; // See state.rs#L99

    if input.len() != BASE_ACCOUNT_LENGTH {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(input[ACCOUNT_INITIALIZED_INDEX] != 0)
}

fn get_extension_bytes<S: BaseState, V: Extension>(tlv_data: &[u8]) -> Result<&[u8], ProgramError> {
    if V::TYPE.get_account_type() != S::ACCOUNT_TYPE {
        return Err(ProgramError::InvalidAccountData);
    }
    let TlvIndices {
        type_start: _,
        length_start,
        value_start,
    } = get_extension_indices::<V>(tlv_data, false)?;
    // get_extension_indices has checked that tlv_data is long enough to include
    // these indices
    let length = pod_from_bytes::<Length>(&tlv_data[length_start..value_start])?;
    let value_end = value_start.saturating_add(usize::from(*length));
    if tlv_data.len() < value_end {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(&tlv_data[value_start..value_end])
}

fn get_extension_bytes_mut<S: BaseState, V: Extension>(
    tlv_data: &mut [u8],
) -> Result<&mut [u8], ProgramError> {
    if V::TYPE.get_account_type() != S::ACCOUNT_TYPE {
        return Err(ProgramError::InvalidAccountData);
    }
    let TlvIndices {
        type_start: _,
        length_start,
        value_start,
    } = get_extension_indices::<V>(tlv_data, false)?;
    // get_extension_indices has checked that tlv_data is long enough to include
    // these indices
    let length = pod_from_bytes::<Length>(&tlv_data[length_start..value_start])?;
    let value_end = value_start.saturating_add(usize::from(*length));
    if tlv_data.len() < value_end {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(&mut tlv_data[value_start..value_end])
}

/// Calculate the new expected size if the state allocates the given number
/// of bytes for the given extension type.
///
/// Provides the correct answer regardless if the extension is already present
/// in the TLV data.
fn try_get_new_account_len_for_extension_len<S: BaseState, V: Extension>(
    tlv_data: &[u8],
    new_extension_len: usize,
) -> Result<usize, ProgramError> {
    // get the new length used by the extension
    let new_extension_tlv_len = add_type_and_length_to_len(new_extension_len);
    let tlv_info = get_tlv_data_info(tlv_data)?;
    // If we're adding an extension, then we must have at least BASE_ACCOUNT_LENGTH
    // and account type
    let current_len = tlv_info
        .used_len
        .saturating_add(BASE_ACCOUNT_AND_TYPE_LENGTH);
    // get the current length used by the extension
    let current_extension_len = get_extension_bytes::<S, V>(tlv_data)
        .map(|x| add_type_and_length_to_len(x.len()))
        .unwrap_or(0);
    let new_len = current_len
        .saturating_sub(current_extension_len)
        .saturating_add(new_extension_tlv_len);
    Ok(adjust_len_for_multisig(new_len))
}

/// Trait for base state with extension
pub trait BaseStateWithExtensions<S: BaseState> {
    /// Get the buffer containing all extension data
    fn get_tlv_data(&self) -> &[u8];

    /// Fetch the bytes for a TLV entry
    fn get_extension_bytes<V: Extension>(&self) -> Result<&[u8], ProgramError> {
        get_extension_bytes::<S, V>(self.get_tlv_data())
    }

    /// Unpack a portion of the TLV data as the desired type
    fn get_extension<V: Extension + Pod>(&self) -> Result<&V, ProgramError> {
        pod_from_bytes::<V>(self.get_extension_bytes::<V>()?)
    }

    /// Unpacks a portion of the TLV data as the desired variable-length type
    fn get_variable_len_extension<V: Extension + VariableLenPack>(
        &self,
    ) -> Result<V, ProgramError> {
        let data = get_extension_bytes::<S, V>(self.get_tlv_data())?;
        V::unpack_from_slice(data)
    }

    /// Iterates through the TLV entries, returning only the types
    fn get_extension_types(&self) -> Result<Vec<ExtensionType>, ProgramError> {
        get_tlv_data_info(self.get_tlv_data()).map(|x| x.extension_types)
    }

    /// Get just the first extension type, useful to track mixed initializations
    fn get_first_extension_type(&self) -> Result<Option<ExtensionType>, ProgramError> {
        get_first_extension_type(self.get_tlv_data())
    }

    /// Get the total number of bytes used by TLV entries and the base type
    fn try_get_account_len(&self) -> Result<usize, ProgramError> {
        let tlv_info = get_tlv_data_info(self.get_tlv_data())?;
        if tlv_info.extension_types.is_empty() {
            Ok(S::SIZE_OF)
        } else {
            let total_len = tlv_info
                .used_len
                .saturating_add(BASE_ACCOUNT_AND_TYPE_LENGTH);
            Ok(adjust_len_for_multisig(total_len))
        }
    }
    /// Calculate the new expected size if the state allocates the given
    /// fixed-length extension instance.
    /// If the state already has the extension, the resulting account length
    /// will be unchanged.
    fn try_get_new_account_len<V: Extension + Pod>(&self) -> Result<usize, ProgramError> {
        try_get_new_account_len_for_extension_len::<S, V>(
            self.get_tlv_data(),
            pod_get_packed_len::<V>(),
        )
    }

    /// Calculate the new expected size if the state allocates the given
    /// variable-length extension instance.
    fn try_get_new_account_len_for_variable_len_extension<V: Extension + VariableLenPack>(
        &self,
        new_extension: &V,
    ) -> Result<usize, ProgramError> {
        try_get_new_account_len_for_extension_len::<S, V>(
            self.get_tlv_data(),
            new_extension.get_packed_len()?,
        )
    }
}

/// Encapsulates owned immutable base state data (mint or account) with possible
/// extensions
#[derive(Clone, Debug, PartialEq)]
pub struct StateWithExtensionsOwned<S: BaseState> {
    /// Unpacked base data
    pub base: S,
    /// Raw TLV data, deserialized on demand
    tlv_data: Vec<u8>,
}
impl<S: BaseState + Pack> StateWithExtensionsOwned<S> {
    /// Unpack base state, leaving the extension data as a slice
    ///
    /// Fails if the base state is not initialized.
    pub fn unpack(mut input: Vec<u8>) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(&input, S::SIZE_OF)?;
        let mut rest = input.split_off(S::SIZE_OF);
        let base = S::unpack(&input)?;
        if let Some((account_type_index, tlv_start_index)) = type_and_tlv_indices::<S>(&rest)? {
            // type_and_tlv_indices() checks that returned indexes are within range
            let account_type = AccountType::try_from(rest[account_type_index])
                .map_err(|_| ProgramError::InvalidAccountData)?;
            check_account_type::<S>(account_type)?;
            let tlv_data = rest.split_off(tlv_start_index);
            Ok(Self { base, tlv_data })
        } else {
            Ok(Self {
                base,
                tlv_data: vec![],
            })
        }
    }
}

impl<S: BaseState> BaseStateWithExtensions<S> for StateWithExtensionsOwned<S> {
    fn get_tlv_data(&self) -> &[u8] {
        &self.tlv_data
    }
}

/// Encapsulates immutable base state data (mint or account) with possible
/// extensions
#[derive(Debug, PartialEq)]
pub struct StateWithExtensions<'data, S: BaseState + Pack> {
    /// Unpacked base data
    pub base: S,
    /// Slice of data containing all TLV data, deserialized on demand
    tlv_data: &'data [u8],
}
impl<'data, S: BaseState + Pack> StateWithExtensions<'data, S> {
    /// Unpack base state, leaving the extension data as a slice
    ///
    /// Fails if the base state is not initialized.
    pub fn unpack(input: &'data [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at(S::SIZE_OF);
        let base = S::unpack(base_data)?;
        let tlv_data = unpack_tlv_data::<S>(rest)?;
        Ok(Self { base, tlv_data })
    }
}
impl<'a, S: BaseState + Pack> BaseStateWithExtensions<S> for StateWithExtensions<'a, S> {
    fn get_tlv_data(&self) -> &[u8] {
        self.tlv_data
    }
}

/// Encapsulates immutable base state data (mint or account) with possible
/// extensions, where the base state is Pod for zero-copy serde.
#[derive(Debug, PartialEq)]
pub struct PodStateWithExtensions<'data, S: BaseState + Pod> {
    /// Unpacked base data
    pub base: &'data S,
    /// Slice of data containing all TLV data, deserialized on demand
    tlv_data: &'data [u8],
}
impl<'data, S: BaseState + Pod> PodStateWithExtensions<'data, S> {
    /// Unpack base state, leaving the extension data as a slice
    ///
    /// Fails if the base state is not initialized.
    pub fn unpack(input: &'data [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at(S::SIZE_OF);
        let base = pod_from_bytes::<S>(base_data)?;
        if !base.is_initialized() {
            Err(ProgramError::UninitializedAccount)
        } else {
            let tlv_data = unpack_tlv_data::<S>(rest)?;
            Ok(Self { base, tlv_data })
        }
    }
}
impl<'a, S: BaseState + Pod> BaseStateWithExtensions<S> for PodStateWithExtensions<'a, S> {
    fn get_tlv_data(&self) -> &[u8] {
        self.tlv_data
    }
}

/// Trait for mutable base state with extension
pub trait BaseStateWithExtensionsMut<S: BaseState>: BaseStateWithExtensions<S> {
    /// Get the underlying TLV data as mutable
    fn get_tlv_data_mut(&mut self) -> &mut [u8];

    /// Get the underlying account type as mutable
    fn get_account_type_mut(&mut self) -> &mut [u8];

    /// Unpack a portion of the TLV data as the base mutable bytes
    fn get_extension_bytes_mut<V: Extension>(&mut self) -> Result<&mut [u8], ProgramError> {
        get_extension_bytes_mut::<S, V>(self.get_tlv_data_mut())
    }

    /// Unpack a portion of the TLV data as the desired type that allows
    /// modifying the type
    fn get_extension_mut<V: Extension + Pod>(&mut self) -> Result<&mut V, ProgramError> {
        pod_from_bytes_mut::<V>(self.get_extension_bytes_mut::<V>()?)
    }

    /// Packs a variable-length extension into its appropriate data segment.
    /// Fails if space hasn't already been allocated for the given extension
    fn pack_variable_len_extension<V: Extension + VariableLenPack>(
        &mut self,
        extension: &V,
    ) -> Result<(), ProgramError> {
        let data = self.get_extension_bytes_mut::<V>()?;
        // NOTE: Do *not* use `pack`, since the length check will cause
        // reallocations to smaller sizes to fail
        extension.pack_into_slice(data)
    }

    /// Packs the default extension data into an open slot if not already found
    /// in the data buffer. If extension is already found in the buffer, it
    /// overwrites the existing extension with the default state if
    /// `overwrite` is set. If extension found, but `overwrite` is not set,
    /// it returns error.
    fn init_extension<V: Extension + Pod + Default>(
        &mut self,
        overwrite: bool,
    ) -> Result<&mut V, ProgramError> {
        let length = pod_get_packed_len::<V>();
        let buffer = self.alloc::<V>(length, overwrite)?;
        let extension_ref = pod_from_bytes_mut::<V>(buffer)?;
        *extension_ref = V::default();
        Ok(extension_ref)
    }

    /// Reallocate and overwrite the TLV entry for the given variable-length
    /// extension.
    ///
    /// Returns an error if the extension is not present, or if there is not
    /// enough space in the buffer.
    fn realloc_variable_len_extension<V: Extension + VariableLenPack>(
        &mut self,
        new_extension: &V,
    ) -> Result<(), ProgramError> {
        let data = self.realloc::<V>(new_extension.get_packed_len()?)?;
        new_extension.pack_into_slice(data)
    }

    /// Reallocate the TLV entry for the given extension to the given number of
    /// bytes.
    ///
    /// If the new length is smaller, it will compact the rest of the buffer and
    /// zero out the difference at the end. If it's larger, it will move the
    /// rest of the buffer data and zero out the new data.
    ///
    /// Returns an error if the extension is not present, or if this is not
    /// enough space in the buffer.
    fn realloc<V: Extension + VariableLenPack>(
        &mut self,
        length: usize,
    ) -> Result<&mut [u8], ProgramError> {
        let tlv_data = self.get_tlv_data_mut();
        let TlvIndices {
            type_start: _,
            length_start,
            value_start,
        } = get_extension_indices::<V>(tlv_data, false)?;
        let tlv_len = get_tlv_data_info(tlv_data).map(|x| x.used_len)?;
        let data_len = tlv_data.len();

        let length_ref = pod_from_bytes_mut::<Length>(&mut tlv_data[length_start..value_start])?;
        let old_length = usize::from(*length_ref);

        // Length check to avoid a panic later in `copy_within`
        if old_length < length {
            let new_tlv_len = tlv_len.saturating_add(length.saturating_sub(old_length));
            if new_tlv_len > data_len {
                return Err(ProgramError::InvalidAccountData);
            }
        }

        // write new length after the check, to avoid getting into a bad situation
        // if trying to recover from an error
        *length_ref = Length::try_from(length)?;

        let old_value_end = value_start.saturating_add(old_length);
        let new_value_end = value_start.saturating_add(length);
        tlv_data.copy_within(old_value_end..tlv_len, new_value_end);
        match old_length.cmp(&length) {
            Ordering::Greater => {
                // realloc to smaller, zero out the end
                let new_tlv_len = tlv_len.saturating_sub(old_length.saturating_sub(length));
                tlv_data[new_tlv_len..tlv_len].fill(0);
            }
            Ordering::Less => {
                // realloc to bigger, zero out the new bytes
                tlv_data[old_value_end..new_value_end].fill(0);
            }
            Ordering::Equal => {} // nothing needed!
        }

        Ok(&mut tlv_data[value_start..new_value_end])
    }

    /// Allocate the given number of bytes for the given variable-length
    /// extension and write its contents into the TLV buffer.
    ///
    /// This can only be used for variable-sized types, such as `String` or
    /// `Vec`. `Pod` types must use `init_extension`
    fn init_variable_len_extension<V: Extension + VariableLenPack>(
        &mut self,
        extension: &V,
        overwrite: bool,
    ) -> Result<(), ProgramError> {
        let data = self.alloc::<V>(extension.get_packed_len()?, overwrite)?;
        extension.pack_into_slice(data)
    }

    /// Allocate some space for the extension in the TLV data
    fn alloc<V: Extension>(
        &mut self,
        length: usize,
        overwrite: bool,
    ) -> Result<&mut [u8], ProgramError> {
        if V::TYPE.get_account_type() != S::ACCOUNT_TYPE {
            return Err(ProgramError::InvalidAccountData);
        }
        let tlv_data = self.get_tlv_data_mut();
        let TlvIndices {
            type_start,
            length_start,
            value_start,
        } = get_extension_indices::<V>(tlv_data, true)?;

        if tlv_data[type_start..].len() < add_type_and_length_to_len(length) {
            return Err(ProgramError::InvalidAccountData);
        }
        let extension_type = ExtensionType::try_from(&tlv_data[type_start..length_start])?;

        if extension_type == ExtensionType::Uninitialized || overwrite {
            // write extension type
            let extension_type_array: [u8; 2] = V::TYPE.into();
            let extension_type_ref = &mut tlv_data[type_start..length_start];
            extension_type_ref.copy_from_slice(&extension_type_array);
            // write length
            let length_ref =
                pod_from_bytes_mut::<Length>(&mut tlv_data[length_start..value_start])?;

            // check that the length is the same if we're doing an alloc
            // with overwrite, otherwise a realloc should be done
            if overwrite && extension_type == V::TYPE && usize::from(*length_ref) != length {
                return Err(TokenError::InvalidLengthForAlloc.into());
            }

            *length_ref = Length::try_from(length)?;

            let value_end = value_start.saturating_add(length);
            Ok(&mut tlv_data[value_start..value_end])
        } else {
            // extension is already initialized, but no overwrite permission
            Err(TokenError::ExtensionAlreadyInitialized.into())
        }
    }

    /// If `extension_type` is an Account-associated ExtensionType that requires
    /// initialization on InitializeAccount, this method packs the default
    /// relevant Extension of an ExtensionType into an open slot if not
    /// already found in the data buffer, otherwise overwrites the
    /// existing extension with the default state. For all other ExtensionTypes,
    /// this is a no-op.
    fn init_account_extension_from_type(
        &mut self,
        extension_type: ExtensionType,
    ) -> Result<(), ProgramError> {
        if extension_type.get_account_type() != AccountType::Account {
            return Ok(());
        }
        match extension_type {
            // ExtensionType::TransferFeeAmount => {
            //     self.init_extension::<TransferFeeAmount>(true).map(|_| ())
            // }
            ExtensionType::ImmutableOwner => {
                self.init_extension::<ImmutableOwner>(true).map(|_| ())
            }
            ExtensionType::NonTransferableAccount => self
                .init_extension::<NonTransferableAccount>(true)
                .map(|_| ()),
            // ExtensionType::TransferHookAccount => {
            //     self.init_extension::<TransferHookAccount>(true).map(|_| ())
            // }
            // // ConfidentialTransfers are currently opt-in only, so this is a no-op for extra safety
            // // on InitializeAccount
            // ExtensionType::ConfidentialTransferAccount => Ok(()),
            // #[cfg(test)]
            // ExtensionType::AccountPaddingTest => {
            //     self.init_extension::<AccountPaddingTest>(true).map(|_| ())
            // }
            _ => unreachable!(),
        }
    }

    /// Write the account type into the buffer, done during the base
    /// state initialization
    /// Noops if there is no room for an extension in the account, needed for
    /// pure base mints / accounts.
    fn init_account_type(&mut self) -> Result<(), ProgramError> {
        let first_extension_type = self.get_first_extension_type()?;
        let account_type = self.get_account_type_mut();
        if !account_type.is_empty() {
            if let Some(extension_type) = first_extension_type {
                let account_type = extension_type.get_account_type();
                if account_type != S::ACCOUNT_TYPE {
                    return Err(TokenError::ExtensionBaseMismatch.into());
                }
            }
            account_type[0] = S::ACCOUNT_TYPE.into();
        }
        Ok(())
    }

    /// Check that the account type on the account (if initialized) matches the
    /// account type for any extensions initialized on the TLV data
    fn check_account_type_matches_extension_type(&self) -> Result<(), ProgramError> {
        if let Some(extension_type) = self.get_first_extension_type()? {
            let account_type = extension_type.get_account_type();
            if account_type != S::ACCOUNT_TYPE {
                return Err(TokenError::ExtensionBaseMismatch.into());
            }
        }
        Ok(())
    }
}

/// Encapsulates mutable base state data (mint or account) with possible
/// extensions
#[derive(Debug, PartialEq)]
pub struct StateWithExtensionsMut<'data, S: BaseState> {
    /// Unpacked base data
    pub base: S,
    /// Raw base data
    base_data: &'data mut [u8],
    /// Writable account type
    account_type: &'data mut [u8],
    /// Slice of data containing all TLV data, deserialized on demand
    tlv_data: &'data mut [u8],
}
impl<'data, S: BaseState + Pack> StateWithExtensionsMut<'data, S> {
    /// Unpack base state, leaving the extension data as a mutable slice
    ///
    /// Fails if the base state is not initialized.
    pub fn unpack(input: &'data mut [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at_mut(S::SIZE_OF);
        let base = S::unpack(base_data)?;
        let (account_type, tlv_data) = unpack_type_and_tlv_data_mut::<S>(rest)?;
        Ok(Self {
            base,
            base_data,
            account_type,
            tlv_data,
        })
    }

    /// Unpack an uninitialized base state, leaving the extension data as a
    /// mutable slice
    ///
    /// Fails if the base state has already been initialized.
    pub fn unpack_uninitialized(input: &'data mut [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at_mut(S::SIZE_OF);
        let base = S::unpack_unchecked(base_data)?;
        if base.is_initialized() {
            return Err(TokenError::AlreadyInUse.into());
        }
        let (account_type, tlv_data) = unpack_uninitialized_type_and_tlv_data_mut::<S>(rest)?;
        let state = Self {
            base,
            base_data,
            account_type,
            tlv_data,
        };
        state.check_account_type_matches_extension_type()?;
        Ok(state)
    }

    /// Packs base state data into the base data portion
    pub fn pack_base(&mut self) {
        S::pack_into_slice(&self.base, self.base_data);
    }
}
impl<'a, S: BaseState> BaseStateWithExtensions<S> for StateWithExtensionsMut<'a, S> {
    fn get_tlv_data(&self) -> &[u8] {
        self.tlv_data
    }
}
impl<'a, S: BaseState> BaseStateWithExtensionsMut<S> for StateWithExtensionsMut<'a, S> {
    fn get_tlv_data_mut(&mut self) -> &mut [u8] {
        self.tlv_data
    }
    fn get_account_type_mut(&mut self) -> &mut [u8] {
        self.account_type
    }
}

/// Encapsulates mutable base state data (mint or account) with possible
/// extensions, where the base state is Pod for zero-copy serde.
#[derive(Debug, PartialEq)]
pub struct PodStateWithExtensionsMut<'data, S: BaseState> {
    /// Unpacked base data
    pub base: &'data mut S,
    /// Writable account type
    account_type: &'data mut [u8],
    /// Slice of data containing all TLV data, deserialized on demand
    tlv_data: &'data mut [u8],
}
impl<'data, S: BaseState + Pod> PodStateWithExtensionsMut<'data, S> {
    /// Unpack base state, leaving the extension data as a mutable slice
    ///
    /// Fails if the base state is not initialized.
    pub fn unpack(input: &'data mut [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at_mut(S::SIZE_OF);
        let base = pod_from_bytes_mut::<S>(base_data)?;
        if !base.is_initialized() {
            Err(ProgramError::UninitializedAccount)
        } else {
            let (account_type, tlv_data) = unpack_type_and_tlv_data_mut::<S>(rest)?;
            Ok(Self {
                base,
                account_type,
                tlv_data,
            })
        }
    }

    /// Unpack an uninitialized base state, leaving the extension data as a
    /// mutable slice
    ///
    /// Fails if the base state has already been initialized.
    pub fn unpack_uninitialized(input: &'data mut [u8]) -> Result<Self, ProgramError> {
        check_min_len_and_not_multisig(input, S::SIZE_OF)?;
        let (base_data, rest) = input.split_at_mut(S::SIZE_OF);
        let base = pod_from_bytes_mut::<S>(base_data)?;
        if base.is_initialized() {
            return Err(TokenError::AlreadyInUse.into());
        }
        let (account_type, tlv_data) = unpack_uninitialized_type_and_tlv_data_mut::<S>(rest)?;
        let state = Self {
            base,
            account_type,
            tlv_data,
        };
        state.check_account_type_matches_extension_type()?;
        Ok(state)
    }
}

impl<'a, S: BaseState> BaseStateWithExtensions<S> for PodStateWithExtensionsMut<'a, S> {
    fn get_tlv_data(&self) -> &[u8] {
        self.tlv_data
    }
}
impl<'a, S: BaseState> BaseStateWithExtensionsMut<S> for PodStateWithExtensionsMut<'a, S> {
    fn get_tlv_data_mut(&mut self) -> &mut [u8] {
        self.tlv_data
    }
    fn get_account_type_mut(&mut self) -> &mut [u8] {
        self.account_type
    }
}

fn unpack_tlv_data<S: BaseState>(rest: &[u8]) -> Result<&[u8], ProgramError> {
    if let Some((account_type_index, tlv_start_index)) = type_and_tlv_indices::<S>(rest)? {
        // type_and_tlv_indices() checks that returned indexes are within range
        let account_type = AccountType::try_from(rest[account_type_index])
            .map_err(|_| ProgramError::InvalidAccountData)?;
        check_account_type::<S>(account_type)?;
        Ok(&rest[tlv_start_index..])
    } else {
        Ok(&[])
    }
}

fn unpack_type_and_tlv_data_with_check_mut<
    S: BaseState,
    F: Fn(AccountType) -> Result<(), ProgramError>,
>(
    rest: &mut [u8],
    check_fn: F,
) -> Result<(&mut [u8], &mut [u8]), ProgramError> {
    if let Some((account_type_index, tlv_start_index)) = type_and_tlv_indices::<S>(rest)? {
        // type_and_tlv_indices() checks that returned indexes are within range
        let account_type = AccountType::try_from(rest[account_type_index])
            .map_err(|_| ProgramError::InvalidAccountData)?;
        check_fn(account_type)?;
        let (account_type, tlv_data) = rest.split_at_mut(tlv_start_index);
        Ok((
            &mut account_type[account_type_index..tlv_start_index],
            tlv_data,
        ))
    } else {
        Ok((&mut [], &mut []))
    }
}

fn unpack_type_and_tlv_data_mut<S: BaseState>(
    rest: &mut [u8],
) -> Result<(&mut [u8], &mut [u8]), ProgramError> {
    unpack_type_and_tlv_data_with_check_mut::<S, _>(rest, check_account_type::<S>)
}

fn unpack_uninitialized_type_and_tlv_data_mut<S: BaseState>(
    rest: &mut [u8],
) -> Result<(&mut [u8], &mut [u8]), ProgramError> {
    unpack_type_and_tlv_data_with_check_mut::<S, _>(rest, |account_type| {
        if account_type != AccountType::Uninitialized {
            Err(ProgramError::InvalidAccountData)
        } else {
            Ok(())
        }
    })
}

/// If AccountType is uninitialized, set it to the BaseState's ACCOUNT_TYPE;
/// if AccountType is already set, check is set correctly for BaseState
/// This method assumes that the `base_data` has already been packed with data
/// of the desired type.
pub fn set_account_type<S: BaseState>(input: &mut [u8]) -> Result<(), ProgramError> {
    check_min_len_and_not_multisig(input, S::SIZE_OF)?;
    let (base_data, rest) = input.split_at_mut(S::SIZE_OF);
    if S::ACCOUNT_TYPE == AccountType::Account && !is_initialized_account(base_data)? {
        return Err(ProgramError::InvalidAccountData);
    }
    if let Some((account_type_index, _tlv_start_index)) = type_and_tlv_indices::<S>(rest)? {
        let mut account_type = AccountType::try_from(rest[account_type_index])
            .map_err(|_| ProgramError::InvalidAccountData)?;
        if account_type == AccountType::Uninitialized {
            rest[account_type_index] = S::ACCOUNT_TYPE.into();
            account_type = S::ACCOUNT_TYPE;
        }
        check_account_type::<S>(account_type)?;
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Different kinds of accounts. Note that `Mint`, `Account`, and `Multisig`
/// types are determined exclusively by the size of the account, and are not
/// included in the account data. `AccountType` is only included if extensions
/// have been initialized.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, TryFromPrimitive, IntoPrimitive)]
pub enum AccountType {
    /// Marker for 0 data
    Uninitialized,
    /// Mint account with additional extensions
    Mint,
    /// Token holding account with additional extensions
    Account,
}
impl Default for AccountType {
    fn default() -> Self {
        Self::Uninitialized
    }
}

/// Extensions that can be applied to mints or accounts.  Mint extensions must
/// only be applied to mint accounts, and account extensions must only be
/// applied to token holding accounts.
#[repr(u16)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, PartialEq, TryFromPrimitive, IntoPrimitive)]
pub enum ExtensionType {
    /// Used as padding if the account size would otherwise be 355, same as a
    /// multisig
    Uninitialized,
    /// Includes transfer fee rate info and accompanying authorities to withdraw
    /// and set the fee
    TransferFeeConfig,
    /// Includes withheld transfer fees
    TransferFeeAmount,
    /// Includes an optional mint close authority
    MintCloseAuthority,
    /// Auditor configuration for confidential transfers
    ConfidentialTransferMint,
    /// State for confidential transfers
    ConfidentialTransferAccount,
    /// Specifies the default Account::state for new Accounts
    DefaultAccountState,
    /// Indicates that the Account owner authority cannot be changed
    ImmutableOwner,
    /// Require inbound transfers to have memo
    MemoTransfer,
    /// Indicates that the tokens from this mint can't be transferred
    NonTransferable,
    /// Tokens accrue interest over time,
    InterestBearingConfig,
    /// Locks privileged token operations from happening via CPI
    CpiGuard,
    /// Includes an optional permanent delegate
    PermanentDelegate,
    /// Indicates that the tokens in this account belong to a non-transferable
    /// mint
    NonTransferableAccount,
    /// Mint requires a CPI to a program implementing the "transfer hook"
    /// interface
    TransferHook,
    /// Indicates that the tokens in this account belong to a mint with a
    /// transfer hook
    TransferHookAccount,
    /// Includes encrypted withheld fees and the encryption public that they are
    /// encrypted under
    ConfidentialTransferFeeConfig,
    /// Includes confidential withheld transfer fees
    ConfidentialTransferFeeAmount,
    /// Mint contains a pointer to another account (or the same account) that
    /// holds metadata
    MetadataPointer,
    /// Mint contains token-metadata
    TokenMetadata,
    /// Mint contains a pointer to another account (or the same account) that
    /// holds group configurations
    GroupPointer,
    /// Mint contains token group configurations
    TokenGroup,
    /// Mint contains a pointer to another account (or the same account) that
    /// holds group member configurations
    GroupMemberPointer,
    /// Mint contains token group member configurations
    TokenGroupMember,
    /// Mint allowing the minting and burning of confidential tokens
    ConfidentialMintBurn,
}
impl TryFrom<&[u8]> for ExtensionType {
    type Error = ProgramError;
    fn try_from(a: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from(u16::from_le_bytes(
            a.try_into().map_err(|_| ProgramError::InvalidAccountData)?,
        ))
        .map_err(|_| ProgramError::InvalidAccountData)
    }
}
impl From<ExtensionType> for [u8; 2] {
    fn from(a: ExtensionType) -> Self {
        u16::from(a).to_le_bytes()
    }
}
impl ExtensionType {
    /// Returns true if the given extension type is sized
    ///
    /// Most extension types should be sized, so any variable-length extension
    /// types should be added here by hand
    const fn sized(&self) -> bool {
        match self {
            ExtensionType::TokenMetadata => false,

            _ => true,
        }
    }

    /// Get the data length of the type associated with the enum
    ///
    /// Fails if the extension type has a variable length
    fn try_get_type_len(&self) -> Result<usize, ProgramError> {
        if !self.sized() {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(match self {
            ExtensionType::Uninitialized => 0,
            ExtensionType::TransferFeeConfig => pod_get_packed_len::<TransferFeeConfig>(),
            ExtensionType::TransferFeeAmount => pod_get_packed_len::<TransferFeeAmount>(),
            ExtensionType::MintCloseAuthority => pod_get_packed_len::<MintCloseAuthority>(),
            ExtensionType::ImmutableOwner => pod_get_packed_len::<ImmutableOwner>(),
            ExtensionType::ConfidentialTransferMint => {
                unreachable!("unsupported")
            }
            ExtensionType::ConfidentialTransferAccount => {
                unreachable!("unsupported")
            }
            ExtensionType::DefaultAccountState => pod_get_packed_len::<DefaultAccountState>(),
            ExtensionType::MemoTransfer => {
                unreachable!("unsupported")
            }
            ExtensionType::NonTransferable => pod_get_packed_len::<NonTransferable>(),
            ExtensionType::InterestBearingConfig => {
                unreachable!("unsupported")
            }
            ExtensionType::CpiGuard => {
                unreachable!("unsupported")
            }
            ExtensionType::PermanentDelegate => {
                unreachable!("unsupported")
            }
            ExtensionType::NonTransferableAccount => pod_get_packed_len::<NonTransferableAccount>(),
            ExtensionType::TransferHook => {
                unreachable!("unsupported")
            }
            ExtensionType::TransferHookAccount => {
                unreachable!("unsupported")
            }
            ExtensionType::ConfidentialTransferFeeConfig => {
                unreachable!("unsupported")
            }
            ExtensionType::ConfidentialTransferFeeAmount => {
                unreachable!("unsupported")
            }
            ExtensionType::MetadataPointer => {
                unreachable!("unsupported")
            }
            ExtensionType::TokenMetadata => unreachable!(),
            ExtensionType::GroupPointer => {
                unreachable!("unsupported")
            }
            ExtensionType::TokenGroup => {
                unreachable!("unsupported")
            }
            ExtensionType::GroupMemberPointer => {
                unreachable!("unsupported")
            }
            ExtensionType::TokenGroupMember => {
                unreachable!("unsupported")
            }
            ExtensionType::ConfidentialMintBurn => {
                unreachable!("unsupported")
            }
        })
    }

    /// Get the TLV length for an ExtensionType
    ///
    /// Fails if the extension type has a variable length
    fn try_get_tlv_len(&self) -> Result<usize, ProgramError> {
        Ok(add_type_and_length_to_len(self.try_get_type_len()?))
    }

    /// Get the TLV length for a set of ExtensionTypes
    ///
    /// Fails if any of the extension types has a variable length
    fn try_get_total_tlv_len(extension_types: &[Self]) -> Result<usize, ProgramError> {
        // dedupe extensions
        let mut extensions = vec![];
        for extension_type in extension_types {
            if !extensions.contains(&extension_type) {
                extensions.push(extension_type);
            }
        }
        extensions.iter().map(|e| e.try_get_tlv_len()).sum()
    }

    /// Get the required account data length for the given ExtensionTypes
    ///
    /// Fails if any of the extension types has a variable length
    pub fn try_calculate_account_len<S: BaseState>(
        extension_types: &[Self],
    ) -> Result<usize, ProgramError> {
        if extension_types.is_empty() {
            Ok(S::SIZE_OF)
        } else {
            let extension_size = Self::try_get_total_tlv_len(extension_types)?;
            let total_len = extension_size.saturating_add(BASE_ACCOUNT_AND_TYPE_LENGTH);
            Ok(adjust_len_for_multisig(total_len))
        }
    }

    /// Get the associated account type
    pub fn get_account_type(&self) -> AccountType {
        match self {
            ExtensionType::Uninitialized => AccountType::Uninitialized,
            ExtensionType::TransferFeeConfig
            | ExtensionType::MintCloseAuthority
            | ExtensionType::ConfidentialTransferMint
            | ExtensionType::DefaultAccountState
            | ExtensionType::NonTransferable
            | ExtensionType::InterestBearingConfig
            | ExtensionType::PermanentDelegate
            | ExtensionType::TransferHook
            | ExtensionType::ConfidentialTransferFeeConfig
            | ExtensionType::MetadataPointer
            | ExtensionType::TokenMetadata
            | ExtensionType::GroupPointer
            | ExtensionType::TokenGroup
            | ExtensionType::GroupMemberPointer
            | ExtensionType::ConfidentialMintBurn
            | ExtensionType::TokenGroupMember => AccountType::Mint,
            ExtensionType::ImmutableOwner
            | ExtensionType::TransferFeeAmount
            | ExtensionType::ConfidentialTransferAccount
            | ExtensionType::MemoTransfer
            | ExtensionType::NonTransferableAccount
            | ExtensionType::TransferHookAccount
            | ExtensionType::CpiGuard
            | ExtensionType::ConfidentialTransferFeeAmount => AccountType::Account,
        }
    }

    /// Based on a set of AccountType::Mint ExtensionTypes, get the list of
    /// AccountType::Account ExtensionTypes required on InitializeAccount
    pub fn get_required_init_account_extensions(mint_extension_types: &[Self]) -> Vec<Self> {
        let mut account_extension_types = vec![];
        for extension_type in mint_extension_types {
            match extension_type {
                ExtensionType::TransferFeeConfig => {
                    account_extension_types.push(ExtensionType::TransferFeeAmount);
                }
                ExtensionType::NonTransferable => {
                    account_extension_types.push(ExtensionType::NonTransferableAccount);
                    account_extension_types.push(ExtensionType::ImmutableOwner);
                }
                ExtensionType::TransferHook => {
                    account_extension_types.push(ExtensionType::TransferHookAccount);
                }

                _ => {}
            }
        }
        account_extension_types
    }

    /// Check for invalid combination of mint extensions
    pub fn check_for_invalid_mint_extension_combinations(
        mint_extension_types: &[Self],
    ) -> Result<(), TokenError> {
        let mut transfer_fee_config = false;
        let mut confidential_transfer_mint = false;
        let mut confidential_transfer_fee_config = false;
        let mut confidential_mint_burn = false;

        for extension_type in mint_extension_types {
            match extension_type {
                ExtensionType::TransferFeeConfig => transfer_fee_config = true,
                ExtensionType::ConfidentialTransferMint => confidential_transfer_mint = true,
                ExtensionType::ConfidentialTransferFeeConfig => {
                    confidential_transfer_fee_config = true
                }
                ExtensionType::ConfidentialMintBurn => confidential_mint_burn = true,
                _ => (),
            }
        }

        if confidential_transfer_fee_config && !(transfer_fee_config && confidential_transfer_mint)
        {
            return Err(TokenError::InvalidExtensionCombination);
        }

        if transfer_fee_config && confidential_transfer_mint && !confidential_transfer_fee_config {
            return Err(TokenError::InvalidExtensionCombination);
        }

        if confidential_mint_burn && !confidential_transfer_mint {
            return Err(TokenError::InvalidExtensionCombination);
        }

        Ok(())
    }
}

/// Trait for base states, specifying the associated enum
pub trait BaseState: PackedSizeOf + IsInitialized {
    /// Associated extension type enum, checked at the start of TLV entries
    const ACCOUNT_TYPE: AccountType;
}
impl BaseState for Account {
    const ACCOUNT_TYPE: AccountType = AccountType::Account;
}
impl BaseState for Mint {
    const ACCOUNT_TYPE: AccountType = AccountType::Mint;
}
impl BaseState for PodAccount {
    const ACCOUNT_TYPE: AccountType = AccountType::Account;
}
impl BaseState for PodMint {
    const ACCOUNT_TYPE: AccountType = AccountType::Mint;
}

/// Trait to be implemented by all extension states, specifying which extension
/// and account type they are associated with
pub trait Extension {
    /// Associated extension type enum, checked at the start of TLV entries
    const TYPE: ExtensionType;
}
