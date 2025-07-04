use alloc::vec::Vec;
use fluentbase_sdk::{
    calc_create4_address,
    keccak256,
    Address,
    Bytes,
    ExitCode,
    IsAccountEmpty,
    IsColdAccess,
    MetadataAPI,
    SyscallResult,
    PRECOMPILE_SVM_RUNTIME,
    U256,
};
use hashbrown::HashMap;

pub struct MemStorage {
    in_memory_metadata: HashMap<Address, Vec<u8>>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self {
            in_memory_metadata: Default::default(),
        }
    }

    #[allow(unused)]
    pub fn clear(&mut self) {
        self.in_memory_metadata.clear();
    }
}

impl MetadataAPI for MemStorage {
    fn metadata_write(
        &mut self,
        address: &Address,
        _offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()> {
        let entry = self.in_memory_metadata.entry(address.clone()).or_default();
        let total_len = metadata.len();
        if entry.len() < total_len {
            entry.resize(total_len, 0);
        }
        entry[..metadata.len()].copy_from_slice(metadata.as_ref());
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
            let total_len = /* offset + */length as usize;
            if data.len() < total_len {
                return SyscallResult::new(Default::default(), 0, 0, ExitCode::Err);
            }
            let chunk = &data[/*offset as usize*/..total_len];
            return SyscallResult::new(Bytes::copy_from_slice(chunk), 0, 0, ExitCode::Ok);
        }
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Err)
    }
}
