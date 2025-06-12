use crate::{
    account::AccountSharedData,
    common::{evm_address_from_pubkey, is_evm_pubkey},
    error::SvmError,
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    select_sapi,
    solana_program::{loader_v4, message::legacy, sysvar},
    system_program,
};
use alloc::vec::Vec;
use fluentbase_sdk::{debug_log, ExitCode, SharedAPI, StorageAPI, SyscallResult, U256};
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, bincode::Encode, bincode::Decode)]
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

    pub fn append_many(&mut self, msgs: Vec<legacy::Message>) -> &mut Self {
        self.messages.extend(msgs);
        self
    }
}

lazy_static! {
    pub static ref SYSTEM_PROGRAMS_KEYS: HashSet<Pubkey> = {
        let mut set = HashSet::new();
        set.insert(system_program::id());
        set.insert(native_loader::id());
        // set.insert(bpf_loader_upgradeable::id());
        set.insert(loader_v4::id());
        set.insert(sysvar::clock::id());
        set.insert(sysvar::rent::id());
        set
    };
}

pub(crate) fn extract_account_data_or_default<SAPI: StorageAPI>(
    sapi: &SAPI,
    account_key: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    Ok(storage_read_account_data(sapi, account_key)
        .unwrap_or_else(|_e| AccountSharedData::new(0, 0, &system_program::id())))
}

pub(crate) fn load_program_account<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &SDK,
    sapi: &Option<&mut SAPI>,
    program_accounts: &mut Vec<(Pubkey, AccountSharedData)>,
    account_key: &Pubkey,
) -> Result<bool, SvmError> {
    let program_account_idx = program_accounts
        .iter()
        .position(|(pk, _)| pk == account_key);
    if program_account_idx.is_some() {
        return Ok(false);
    }
    let program_account = select_sapi!(sapi, sdk, |s| {
        extract_account_data_or_default(s, account_key)
    })?;
    // TODO do we need this check?
    // if !program_account.executable() {
    //     return Err(TransactionError::InvalidProgramForExecution.into());
    // }
    program_accounts.push((account_key.clone(), program_account));
    Ok(true)
}

pub(crate) fn flush_accounts<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    sapi: &mut Option<&mut SAPI>,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) -> Result<(), SvmError> {
    for (pk, data) in accounts {
        debug_log!(
            "flushing account (sdk?:{}) {:?} ({:x?}) is_evm_pubkey:{} address {}",
            sapi.is_none(),
            pk,
            pk.to_bytes(),
            is_evm_pubkey(&pk),
            evm_address_from_pubkey::<false>(&pk)?
        );
        select_sapi!(sapi, sdk, |storage| {
            storage_write_account_data(storage, pk, data)
        })?;
    }
    Ok(())
}

// TODO
pub fn process_svm_error(svm_error: SvmError) -> (HashMap<Pubkey, AccountSharedData>, i32) {
    match svm_error {
        SvmError::TransactionError(_err) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::BincodeEncodeError(_err) => {
            (Default::default(), ExitCode::UnknownError.into_i32())
        }
        SvmError::BincodeDecodeError(_err) => {
            (Default::default(), ExitCode::UnknownError.into_i32())
        }
        SvmError::ExitCode(_err) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::InstructionError(_err) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::ElfError(_) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::EbpfError(_) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::SyscallError(_) => (Default::default(), ExitCode::UnknownError.into_i32()),
        SvmError::RuntimeError(_) => (Default::default(), ExitCode::UnknownError.into_i32()),
    }
}

pub fn process_svm_result(
    result: Result<HashMap<Pubkey, AccountSharedData>, SvmError>,
) -> (HashMap<Pubkey, AccountSharedData>, i32) {
    match result {
        Ok(v) => (v, ExitCode::Ok.into_i32()),
        Err(err) => process_svm_error(err),
    }
}

pub struct MemStorage {
    in_memory_storage: HashMap<U256, U256>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self {
            in_memory_storage: HashMap::new(),
        }
    }
}
impl StorageAPI for MemStorage {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        self.in_memory_storage.insert(slot, value);
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let result = self.in_memory_storage.get(slot).cloned();
        SyscallResult::new(result.unwrap_or_default(), 0, 0, ExitCode::Ok)
    }
}
