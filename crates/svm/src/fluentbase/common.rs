use crate::{
    account::AccountSharedData,
    error::SvmError,
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    select_api,
    solana_program::{loader_v4, message::legacy, sysvar},
    system_program,
};
use alloc::{string::String, vec::Vec};
use core::fmt::{Display, Formatter};
use fluentbase_sdk::{MetadataAPI, SharedAPI};
use hashbrown::{HashMap, HashSet};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
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
    pub static ref SYSTEM_PROGRAMS_KEYS: HashSet<Pubkey> = {
        let mut set = HashSet::new();
        set.insert(system_program::id());
        set.insert(native_loader::id());
        set.insert(loader_v4::id());
        set.insert(sysvar::clock::id());
        set
    };
}

pub(crate) fn extract_account_data_or_default<API: MetadataAPI>(
    api: &API,
    account_key: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    Ok(storage_read_account_data(api, account_key)
        .unwrap_or_else(|_e| AccountSharedData::new(0, 0, &system_program::id())))
}

pub(crate) fn flush_accounts<SDK: SharedAPI, API: MetadataAPI>(
    sdk: &mut SDK,
    api: &mut Option<&mut API>,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) -> Result<(), SvmError> {
    for (pk, data) in accounts {
        select_api!(api, sdk, |storage| {
            storage_write_account_data(storage, pk, data)
        })?;
    }
    Ok(())
}

impl Display for SvmError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            SvmError::TransactionError(e) => {
                write!(f, "SvmError::TransactionError:{}", e)
            }
            SvmError::BincodeEncodeError(e) => {
                write!(f, "SvmError::BincodeEncodeError:{}", e)
            }
            SvmError::BincodeDecodeError(e) => {
                write!(f, "SvmError::BincodeDecodeError:{}", e)
            }
            SvmError::InstructionError(e) => {
                write!(f, "SvmError::InstructionError:{}", e)
            }
            SvmError::ElfError(e) => {
                write!(f, "SvmError::ElfError:{}", e)
            }
            SvmError::EbpfError(e) => {
                write!(f, "SvmError::EbpfError:{}", e)
            }
            SvmError::SyscallError(e) => {
                write!(f, "SvmError::SyscallError:{}", e)
            }
            SvmError::RuntimeError(e) => {
                write!(f, "SvmError::RuntimeError:{}", e)
            }
            SvmError::ExitCode(e) => {
                write!(f, "SvmError::ExitCode:{}", e)
            }
        }
    }
}

pub fn process_svm_result<T>(result: Result<T, SvmError>) -> Result<T, String> {
    match result {
        Ok(v) => Ok(v),
        Err(ref err) => Err(alloc::format!("{}", &err)),
    }
}
