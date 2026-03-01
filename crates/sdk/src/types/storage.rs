use crate::{Address, StorageAPI, U256};
use alloc::string::{String, ToString};
use fluentbase_types::ExitCode;

pub trait StorageUtils {
    fn storage_short_string(&self, slot: &U256) -> Result<String, ExitCode>;

    fn write_storage_short_string(&mut self, slot: U256, value: &str) -> Result<(), ExitCode>;

    fn storage_address(&self, slot: &U256) -> Result<Address, ExitCode>;

    fn write_storage_address(&mut self, slot: U256, value: Address) -> Result<(), ExitCode>;
}

impl<T: StorageAPI> StorageUtils for T {
    fn storage_short_string(&self, slot: &U256) -> Result<String, ExitCode> {
        let value = self.storage(slot).ok()?.to_be_bytes::<{ U256::BYTES }>();
        let mut value = value.as_ref();
        if let Some(end) = value.iter().position(|c| *c == 0u8) {
            value = &value[..end];
        }
        let result = str::from_utf8(value).unwrap().to_string();
        Ok(result)
    }

    fn write_storage_short_string(&mut self, slot: U256, value: &str) -> Result<(), ExitCode> {
        debug_assert!(
            value.len() <= U256::BYTES,
            "system: short string can't exceed 32 bytes"
        );
        let mut bytes32 = [0u8; U256::BYTES];
        let bytes = value.as_bytes();
        if bytes.len() > U256::BYTES {
            bytes32.copy_from_slice(&bytes[..U256::BYTES]);
        } else {
            bytes32[..bytes.len()].copy_from_slice(bytes);
        }
        let value = U256::from_be_bytes(bytes32);
        self.write_storage(slot, value).ok()
    }

    fn storage_address(&self, slot: &U256) -> Result<Address, ExitCode> {
        let value = self.storage(slot).ok()?;
        Ok(Address::from_word(
            value.to_be_bytes::<{ U256::BYTES }>().into(),
        ))
    }

    fn write_storage_address(&mut self, slot: U256, value: Address) -> Result<(), ExitCode> {
        let value = U256::from_be_bytes(value.into_word().0);
        self.write_storage(slot, value).ok()
    }
}

pub fn storage_mapping_slot() {}

#[cfg(test)]
mod tests {
    use crate::{types::storage::StorageUtils, StorageAPI, U256};
    use fluentbase_types::{ExitCode, SyscallResult};
    use hashbrown::HashMap;

    #[derive(Default)]
    struct TestingStorage(HashMap<U256, U256>);

    impl StorageAPI for TestingStorage {
        fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
            self.0.insert(slot, value);
            SyscallResult::default()
        }

        fn storage(&self, slot: &U256) -> SyscallResult<U256> {
            let result = self.0.get(slot).cloned().unwrap();
            SyscallResult::new(result, 0, 0, ExitCode::Ok)
        }
    }

    #[test]
    fn test_short_string() {
        let mut storage = TestingStorage::default();
        storage
            .write_storage_short_string(U256::ZERO, "Hello, World!")
            .unwrap();
        let value = storage.storage_short_string(&U256::ZERO).unwrap();
        assert_eq!(value, "Hello, World!");
    }
}
