#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ty) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            let typ = <$struct_typ as Default>::default();
            typ.deploy::<fluentbase_sdk::LowLevelSDK>();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            let typ = <$struct_typ as Default>::default();
            typ.main::<fluentbase_sdk::LowLevelSDK>();
        }
    };
}

#[macro_export]
macro_rules! solidity_storage_mapping {
    ($struct_name:ident, $slot:expr) => {
        struct $struct_name<'a, CR: ContextReader, AM: AccountManager> {
            cr: &'a CR,
            am: &'a AM,
        }

        impl<'a, CR: ContextReader, AM: AccountManager> $struct_name<'a, CR, AM> {
            const SLOT: [u8; 32] = $slot;

            pub fn new(cr: &'a CR, am: &'a AM) -> Self {
                Self { cr, am }
            }
            pub fn get_slot(&self) -> [u8; 32] {
                Self::SLOT
            }

            pub fn storage_mapping_key(&self, slot: [u8; 32], value: &[u8]) -> [u8; 32] {
                let mut raw_storage_key: [u8; 64] = [0; 64];
                raw_storage_key[0..32].copy_from_slice(&Self::SLOT);
                raw_storage_key[32..64].copy_from_slice(value);
                let mut storage_key: [u8; 32] = [0; 32];
                LowLevelSDK::keccak256(
                    raw_storage_key.as_ptr(),
                    raw_storage_key.len() as u32,
                    storage_key.as_mut_ptr(),
                );
                storage_key
            }

            pub fn write(&self, key: U256, value: U256) {
                let contract_address = self.cr.contract_address();
                self.am.write_storage(contract_address, key, value);
            }

            pub fn read(&self, key: U256) -> U256 {
                let contract_address = self.cr.contract_address();
                let (value, _is_cold) = self.am.storage(contract_address, key, false);
                U256::from_le_slice(value.as_le_slice())
            }
        }

        impl Default
            for $struct_name<
                'static,
                fluentbase_sdk::GuestContextReader,
                fluentbase_sdk::GuestAccountManager,
            >
        {
            fn default() -> Self {
                Self {
                    cr: &fluentbase_sdk::GuestContextReader::DEFAULT,
                    am: &fluentbase_sdk::GuestAccountManager::DEFAULT,
                }
            }
        }
    };
}
