use crate::{
    account::AccountSharedData,
    error::SvmError,
    helpers::{storage_read_account_data, storage_write_account_data},
    native_loader,
    select_sapi,
    solana_program::{loader_v4, message::legacy, sysvar},
    system_program,
};
use alloc::{string::String, vec::Vec};
use core::fmt::{Display, Formatter};
use fluentbase_sdk::{
    calc_create4_address,
    debug_log_ext,
    keccak256,
    Address,
    Bytes,
    ExitCode,
    IsAccountEmpty,
    IsColdAccess,
    MetadataAPI,
    SharedAPI,
    SyscallResult,
    PRECOMPILE_SVM_RUNTIME,
    U256,
};
use hashbrown::{HashMap, HashSet};
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

pub(crate) fn extract_account_data_or_default<SAPI: MetadataAPI>(
    sapi: &SAPI,
    account_key: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    Ok(storage_read_account_data(sapi, account_key)
        .unwrap_or_else(|_e| AccountSharedData::new(0, 0, &system_program::id())))
}

pub(crate) fn load_program_account<SDK: SharedAPI, SAPI: MetadataAPI>(
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
    program_accounts.push((account_key.clone(), program_account));
    Ok(true)
}

pub(crate) fn flush_accounts<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    sapi: &mut Option<&mut SAPI>,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) -> Result<(), SvmError> {
    for (pk, data) in accounts {
        debug_log_ext!("flushing data (sapi={}) for pk {}", sapi.is_some(), pk);
        select_sapi!(sapi, sdk, |storage| {
            storage_write_account_data(storage, pk, data)
        })?;
    }
    Ok(())
}

impl Display for SvmError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            SvmError::TransactionError(e) => {
                write!(f, "transaction error: {}", e)
            }
            SvmError::BincodeEncodeError(e) => {
                write!(f, "bincode encode error: {}", e)
            }
            SvmError::BincodeDecodeError(e) => {
                write!(f, "bincode decode error{}", e)
            }
            SvmError::InstructionError(e) => {
                write!(f, "instruction error: {}", e)
            }
            SvmError::ElfError(e) => {
                write!(f, "elf error: {}", e)
            }
            SvmError::EbpfError(e) => {
                write!(f, "ebpf error: {}", e)
            }
            SvmError::SyscallError(e) => {
                write!(f, "syscall error: {}", e)
            }
            SvmError::RuntimeError(e) => {
                write!(f, "runtime error: {}", e)
            }
            SvmError::ExitCode(e) => {
                write!(f, "exit code: {}", e)
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

pub struct MemStorage {
    // in_memory_storage: HashMap<U256, U256>,
    in_memory_metadata: HashMap<Address, Vec<u8>>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self {
            // in_memory_storage: Default::default(),
            in_memory_metadata: Default::default(),
        }
    }
}
// impl StorageAPI for MemStorage {
//     fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
//         self.in_memory_storage.insert(slot, value);
//         SyscallResult::new((), 0, 0, ExitCode::Ok)
//     }
//
//     fn storage(&self, slot: &U256) -> SyscallResult<U256> {
//         let result = self.in_memory_storage.get(slot).cloned();
//         SyscallResult::new(result.unwrap_or_default(), 0, 0, ExitCode::Ok)
//     }
// }
impl MetadataAPI for MemStorage {
    fn metadata_write(
        &mut self,
        address: &Address,
        _offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()> {
        let entry = self.in_memory_metadata.entry(address.clone()).or_default();
        let total_len = metadata.len()/* + offset as usize*/;
        if entry.len() < total_len {
            entry.resize(total_len, 0);
        }
        entry[/*offset as usize*/..].copy_from_slice(metadata.as_ref());
        let entry_len = entry.len();
        assert_eq!(
            self.in_memory_metadata
                .entry(address.clone())
                .or_default()
                .len(),
            entry_len,
            "len doesnt match"
        );
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsColdAccess, IsAccountEmpty)> {
        let len = self
            .in_memory_metadata
            .get(address)
            .map_or_else(|| 0, |v| v.len()) as u32;
        // TODO check bool flags
        SyscallResult::new((len, false, false), 0, 0, ExitCode::Ok)
    }

    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()> {
        let derived_metadata_address =
            calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &salt, |v| keccak256(v));
        self.in_memory_metadata
            .insert(derived_metadata_address, metadata.to_vec());
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn metadata_copy(&self, address: &Address, _offset: u32, length: u32) -> SyscallResult<Bytes> {
        if length <= 0 {
            return SyscallResult::new(Default::default(), 0, 0, ExitCode::Ok);
        }
        let data = self.in_memory_metadata.get(address);
        if let Some(data) = data {
            let total_len = (/* offset + */length) as usize;
            if data.len() < total_len {
                return SyscallResult::new(Default::default(), 0, 0, ExitCode::Err);
            }
            let chunk = &data[/*offset as usize*/..total_len];
            return SyscallResult::new(Bytes::copy_from_slice(chunk), 0, 0, ExitCode::Ok);
        }
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Err)
    }
}
