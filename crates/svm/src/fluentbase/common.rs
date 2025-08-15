use crate::common::{evm_balance_from_lamports, lamports_from_evm_balance, pubkey_to_u256};
use crate::{
    account::{AccountSharedData, ReadableAccount},
    common::is_evm_pubkey,
    error::{RuntimeError, SvmError},
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    solana_program::{loader_v4, message::legacy, sysvar},
    system_program,
};
use alloc::{string::String, vec::Vec};
use core::marker::PhantomData;
use fluentbase_sdk::{MetadataAPI, SharedAPI, U256};
use fluentbase_types::{syscall::SyscallResult, ExitCode, MetadataStorageAPI};
use hashbrown::HashMap;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct BatchMessage {
    messages: Vec<legacy::Message>,
}

impl BatchMessage {
    pub fn new(cap: Option<usize>) -> Self {
        BatchMessage {
            messages: Vec::with_capacity(cap.unwrap_or(0)),
        }
    }

    pub fn messages(&self) -> &Vec<legacy::Message> {
        &self.messages
    }

    pub fn clear(&mut self) -> &mut Self {
        self.messages.clear();
        self
    }

    pub fn append_one(&mut self, msg: legacy::Message) -> &mut Self {
        self.messages.push(msg);
        self
    }

    pub fn append_many(&mut self, messages: Vec<legacy::Message>) -> &mut Self {
        self.messages.extend(messages);
        self
    }
}

lazy_static! {
    pub static ref SYSTEM_PROGRAMS_KEYS: Vec<Pubkey> = {
        // let mut items = HashSet::new();
        // items.insert(system_program::id());
        // items.insert(native_loader::id());
        // items.insert(loader_v4::id());
        // items.insert(sysvar::clock::id());
        // items
        use alloc::vec;
        vec![
            system_program::id(),
            native_loader::id(),
            loader_v4::id(),
            sysvar::clock::id(),
        ]
    };
}

pub(crate) fn extract_account_data_or_default<API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    pk: &Pubkey,
) -> AccountSharedData {
    storage_read_account_data(api, pk).unwrap_or_else(|_e| {
        let lamports = GlobalBalance::get(api, pk);
        AccountSharedData::new(lamports, 0, &system_program::id())
    })
}

/// Stores provided accounts using specified storage api or alt api
/// Filters out system accounts if set
/// Returns error if some accounts are not evm compatible
pub(crate) fn flush_accounts<const SKIP_SYS_ACCS: bool, SDK: SharedAPI>(
    sdk: &mut SDK,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) -> Result<u64, SvmError> {
    let mut accounts_flushed = 0;
    for (pk, account_data) in accounts {
        if SKIP_SYS_ACCS && SYSTEM_PROGRAMS_KEYS.contains(&pk) {
            continue;
        }
        // if !is_evm_pubkey(&pk) {
        //     return Err(SvmError::RuntimeError(RuntimeError::InvalidPrefix));
        // }
        storage_write_account_data(sdk, pk, account_data)?;
        accounts_flushed += 1;
    }
    Ok(accounts_flushed)
}

pub fn process_svm_result<T>(result: Result<T, SvmError>) -> Result<T, String> {
    match result {
        Ok(v) => Ok(v),
        Err(ref err) => Err(alloc::format!("{}", &err)),
    }
}

pub struct GlobalBalance<API: MetadataStorageAPI> {
    _phantom_data: PhantomData<API>,
}

impl<API: MetadataStorageAPI> GlobalBalance<API> {
    pub fn new() -> Self {
        Self {
            _phantom_data: Default::default(),
        }
    }

    pub fn get(sdk: &API, pk: &Pubkey) -> u64 {
        let slot = pubkey_to_u256(pk);
        let balance_current = sdk
            .metadata_storage_read(&slot)
            .expect("failed to read current balance")
            .data;

        lamports_from_evm_balance(balance_current)
    }
    pub fn change<const ADD_OR_SUB: bool>(
        sdk: &mut API,
        pk: &Pubkey,
        lamports_change: u64,
    ) -> Result<(), SvmError> {
        let slot = pubkey_to_u256(pk);
        let balance_current = sdk
            .metadata_storage_read(&slot)
            .expect("failed to read current balance")
            .data;
        let balance_change = evm_balance_from_lamports(lamports_change);

        let balance_new = if ADD_OR_SUB {
            balance_current.checked_add(balance_change)
        } else {
            balance_current.checked_sub(balance_change)
        };
        if let Some(balance_new) = balance_new {
            sdk.metadata_storage_write(&slot, balance_new)
                .expect("failed to write updated balance");
        } else {
            return Err(ExitCode::IntegerOverflow.into());
        }

        Ok(())
    }
    pub fn transfer(
        sdk: &mut API,
        pk_from: &Pubkey,
        pk_to: &Pubkey,
        lamports_change: u64,
    ) -> Result<(), SvmError> {
        let slot_from = pubkey_to_u256(pk_from);
        let slot_to = pubkey_to_u256(pk_to);
        let balance_from_current = sdk
            .metadata_storage_read(&slot_from)
            .expect("failed to read current from balance")
            .data;
        let balance_to_current = sdk
            .metadata_storage_read(&slot_to)
            .expect("failed to read current to balance")
            .data;
        let balance_change = evm_balance_from_lamports(lamports_change);

        let Some(balance_from_new) = balance_from_current.checked_sub(balance_change) else {
            return Err(ExitCode::IntegerOverflow.into());
        };
        let Some(balance_to_new) = balance_to_current.checked_add(balance_change) else {
            return Err(ExitCode::IntegerOverflow.into());
        };
        sdk.metadata_storage_write(&slot_from, balance_from_new)
            .expect("failed to write updated balance");
        sdk.metadata_storage_write(&slot_to, balance_to_new)
            .expect("failed to write updated balance");

        Ok(())
    }
}
