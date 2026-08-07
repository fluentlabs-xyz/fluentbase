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
        // Stored words are not guaranteed to be well-formed: they can come from genesis,
        // a legacy layout, or a raw storage write. Report malformed bytes instead of
        // panicking, which would permanently brick every reader of this slot.
        let result = str::from_utf8(value)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?
            .to_string();
        Ok(result)
    }

    fn write_storage_short_string(&mut self, slot: U256, value: &str) -> Result<(), ExitCode> {
        // Reject before mutating storage. Truncating to 32 bytes can split a UTF-8 code
        // point and persist bytes that no reader can decode.
        if value.len() > U256::BYTES {
            return Err(ExitCode::MalformedBuiltinParams);
        }
        let mut bytes32 = [0u8; U256::BYTES];
        bytes32[..value.len()].copy_from_slice(value.as_bytes());
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
    use alloc::format;
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
            let result = self.0.get(slot).cloned().unwrap_or_default();
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

    #[test]
    fn test_short_string_accepts_exactly_32_bytes() {
        let mut storage = TestingStorage::default();
        let ascii = "a".repeat(U256::BYTES);
        storage
            .write_storage_short_string(U256::ZERO, &ascii)
            .unwrap();
        assert_eq!(storage.storage_short_string(&U256::ZERO).unwrap(), ascii);

        // 32 bytes of multibyte text is also a valid boundary (8 * 4-byte code points).
        let multibyte = "😀".repeat(8);
        assert_eq!(multibyte.len(), U256::BYTES);
        storage
            .write_storage_short_string(U256::ONE, &multibyte)
            .unwrap();
        assert_eq!(storage.storage_short_string(&U256::ONE).unwrap(), multibyte);
    }

    #[test]
    fn test_short_string_rejects_overlong_without_mutating() {
        // 33 ASCII bytes, and 33 bytes whose 32-byte prefix would split a code point.
        for overlong in ["a".repeat(U256::BYTES + 1), format!("{}é", "a".repeat(31))] {
            assert_eq!(overlong.len(), U256::BYTES + 1);
            let mut storage = TestingStorage::default();
            assert_eq!(
                storage.write_storage_short_string(U256::ZERO, &overlong),
                Err(ExitCode::MalformedBuiltinParams)
            );
            assert!(
                storage.0.is_empty(),
                "rejected write must not touch storage"
            );
        }
    }

    #[test]
    fn test_short_string_read_of_malformed_bytes_errors() {
        let mut storage = TestingStorage::default();
        // A lone continuation byte: never valid UTF-8, and reachable via genesis or a
        // legacy raw-bytes layout.
        let mut word = [0u8; U256::BYTES];
        word[0] = 0x80;
        let _ = storage.write_storage(U256::ZERO, U256::from_be_bytes(word));
        assert_eq!(
            storage.storage_short_string(&U256::ZERO),
            Err(ExitCode::MalformedBuiltinParams)
        );
    }
}
