use alloc::vec::Vec;
use fluentbase_sdk::{
    calc_create4_address, debug_log_ext, keccak256, Address, Bytes, ExitCode, IsAccountEmpty,
    IsAccountOwnable, IsColdAccess, MetadataAPI, PRECOMPILE_SVM_RUNTIME, U256,
};
use fluentbase_types::syscall::SyscallResult;
use fluentbase_types::MetadataStorageAPI;
use hashbrown::HashMap;

pub struct MemStorage {
    metadata: HashMap<Address, Vec<u8>>,
    metadata_storage: HashMap<U256, U256>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self {
            metadata: Default::default(),
            metadata_storage: Default::default(),
        }
    }

    #[allow(unused)]
    pub fn clear(&mut self) {
        self.metadata.clear();
        self.metadata_storage.clear();
    }
}

impl MetadataAPI for MemStorage {
    fn metadata_write(
        &mut self,
        address: &Address,
        _offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()> {
        let entry = self.metadata.entry(address.clone()).or_default();
        let total_len = metadata.len();
        if entry.len() < total_len {
            entry.resize(total_len, 0);
        }
        entry[..metadata.len()].copy_from_slice(metadata.as_ref());
        let entry_len = entry.len();
        assert_eq!(
            self.metadata.entry(address.clone()).or_default().len(),
            entry_len,
            "len doesnt match"
        );
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)> {
        let len = self.metadata.get(address).map_or_else(|| 0, |v| v.len()) as u32;
        // TODO check bool flags
        SyscallResult::new((len, false, false, false), 0, 0, ExitCode::Ok)
    }

    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()> {
        let derived_metadata_address =
            calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &salt, |v| keccak256(v));
        self.metadata
            .insert(derived_metadata_address, metadata.to_vec());
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn metadata_copy(&self, address: &Address, _offset: u32, length: u32) -> SyscallResult<Bytes> {
        if length <= 0 {
            return SyscallResult::new(Default::default(), 0, 0, ExitCode::Ok);
        }
        let data = self.metadata.get(address);
        if let Some(data) = data {
            let total_len = length as usize;
            if data.len() < total_len {
                return SyscallResult::new(Default::default(), 0, 0, ExitCode::Err);
            }
            let chunk = &data[..total_len];
            return SyscallResult::new(Bytes::copy_from_slice(chunk), 0, 0, ExitCode::Ok);
        }
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Err)
    }
}

impl MetadataStorageAPI for MemStorage {
    fn metadata_storage_read(&self, slot: &U256) -> SyscallResult<U256> {
        let value = self.metadata_storage.get(slot).cloned().unwrap_or_default();
        debug_log_ext!("read: slot {} value {}", slot, value);
        SyscallResult::new(value, 0, 0, ExitCode::Ok)
    }

    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> SyscallResult<()> {
        debug_log_ext!("write: slot {} value {}", slot, value);
        self.metadata_storage.insert(*slot, value);
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }
}
