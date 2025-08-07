use crate::{
    account::{AccountSharedData, ReadableAccount},
    common::is_evm_pubkey,
    error::{RuntimeError, SvmError},
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    select_api,
    solana_program::{loader_v4, message::legacy, sysvar},
    system_program,
};
use alloc::{string::String, vec::Vec};
use fluentbase_sdk::{debug_log_ext, MetadataAPI, SharedAPI};
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

pub(crate) fn extract_account_data_or_default<API: MetadataAPI>(
    api: &API,
    account_key: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    Ok(storage_read_account_data(api, account_key)
        .unwrap_or_else(|_e| AccountSharedData::new(0, 0, &system_program::id())))
}

/// Stores provided accounts using specified storage api or alt api
/// Filters out system accounts
/// Returns error if some accounts are not evm compatible
pub(crate) fn flush_not_system_accounts<SDK: SharedAPI, API: MetadataAPI>(
    sdk: &mut SDK,
    api: &mut Option<&mut API>,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) -> Result<u64, SvmError> {
    let mut accounts_flushed = 0;
    select_api!(api, sdk, |storage: &mut _| -> Result<(), SvmError> {
        for (pk, account_data) in accounts {
            if SYSTEM_PROGRAMS_KEYS.contains(&pk) {
                continue;
            }
            if !is_evm_pubkey(&pk) {
                return Err(SvmError::RuntimeError(RuntimeError::InvalidPrefix));
            }
            let account_data_owner = account_data.owner();
            debug_log_ext!("account_data_owner {:x?}", account_data_owner);
            storage_write_account_data(storage, pk, account_data)?;
            accounts_flushed += 1;
        }
        Ok(())
    })?;
    debug_log_ext!("accounts flushed {}", accounts_flushed);
    Ok(accounts_flushed)
}

pub fn process_svm_result<T>(result: Result<T, SvmError>) -> Result<T, String> {
    match result {
        Ok(v) => Ok(v),
        Err(ref err) => Err(alloc::format!("{}", &err)),
    }
}
