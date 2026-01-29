// custom_slots.rs
#![allow(dead_code)]
use fluentbase_sdk::{
    derive::{Contract, Storage},
    storage::{StorageAddress, StorageBool, StorageU256},
    Address, SharedAPI, U256,
};

// EIP-1967 storage slots
// https://eips.ethereum.org/EIPS/eip-1967
pub mod eip1967 {
    use fluentbase_sdk::{derive::eip1967_slot, U256};

    pub const IMPLEMENTATION: U256 = eip1967_slot!("eip1967.proxy.implementation");
    pub const ADMIN: U256 = eip1967_slot!("eip1967.proxy.admin");
    pub const BEACON: U256 = eip1967_slot!("eip1967.proxy.beacon");
}

// ERC-7201 namespaced storage slots
// https://eips.ethereum.org/EIPS/eip-7201
pub mod erc7201 {
    use fluentbase_sdk::{derive::erc7201_slot, U256};

    pub const EXAMPLE_MAIN: U256 = erc7201_slot!("example.main");
}

// ============================================================================
// EIP-1967 Proxy Pattern
// ============================================================================

#[derive(Contract)]
pub struct Proxy<SDK> {
    sdk: SDK,

    #[slot(eip1967::IMPLEMENTATION)]
    implementation: StorageAddress,

    #[slot(eip1967::ADMIN)]
    admin: StorageAddress,

    #[slot(eip1967::BEACON)]
    beacon: StorageAddress,
}

impl<SDK: SharedAPI> Proxy<SDK> {
    pub fn get_implementation(&self) -> Address {
        self.implementation_accessor().get(&self.sdk)
    }

    pub fn set_implementation(&mut self, addr: Address) {
        self.implementation_accessor().set(&mut self.sdk, addr);
    }

    pub fn get_admin(&self) -> Address {
        self.admin_accessor().get(&self.sdk)
    }

    pub fn set_admin(&mut self, addr: Address) {
        self.admin_accessor().set(&mut self.sdk, addr);
    }

    pub fn get_beacon(&self) -> Address {
        self.beacon_accessor().get(&self.sdk)
    }

    pub fn set_beacon(&mut self, addr: Address) {
        self.beacon_accessor().set(&mut self.sdk, addr);
    }
}

// ============================================================================
// ERC-7201 Namespaced Storage Pattern
// ============================================================================

#[derive(Storage)]
pub struct NamespacedData {
    owner: StorageAddress,
    counter: StorageU256,
    active: StorageBool,
}

#[derive(Contract)]
pub struct NamespacedContract<SDK> {
    sdk: SDK,

    #[slot(erc7201::EXAMPLE_MAIN)]
    data: NamespacedData,
}

impl<SDK: SharedAPI> NamespacedContract<SDK> {
    pub fn get_owner(&self) -> Address {
        self.data_accessor().owner_accessor().get(&self.sdk)
    }

    pub fn set_owner(&mut self, addr: Address) {
        self.data_accessor()
            .owner_accessor()
            .set(&mut self.sdk, addr);
    }

    pub fn get_counter(&self) -> U256 {
        self.data_accessor().counter_accessor().get(&self.sdk)
    }

    pub fn set_counter(&mut self, value: U256) {
        self.data_accessor()
            .counter_accessor()
            .set(&mut self.sdk, value);
    }

    pub fn get_active(&self) -> bool {
        self.data_accessor().active_accessor().get(&self.sdk)
    }

    pub fn set_active(&mut self, value: bool) {
        self.data_accessor()
            .active_accessor()
            .set(&mut self.sdk, value);
    }
}

// ============================================================================
// Mixed Auto-Layout and Explicit Slots
// ============================================================================

#[derive(Contract)]
pub struct MixedStorage<SDK> {
    sdk: SDK,

    // Auto-layout fields (slots 0, 1)
    owner: StorageAddress,
    counter: StorageU256,

    // Explicit slot (EIP-1967 implementation)
    #[slot(eip1967::IMPLEMENTATION)]
    implementation: StorageAddress,

    // Auto-layout continues (slot 2)
    paused: StorageBool,
}

impl<SDK: SharedAPI> MixedStorage<SDK> {
    pub fn get_owner(&self) -> Address {
        self.owner_accessor().get(&self.sdk)
    }

    pub fn set_owner(&mut self, addr: Address) {
        self.owner_accessor().set(&mut self.sdk, addr);
    }

    pub fn get_counter(&self) -> U256 {
        self.counter_accessor().get(&self.sdk)
    }

    pub fn set_counter(&mut self, value: U256) {
        self.counter_accessor().set(&mut self.sdk, value);
    }

    pub fn get_implementation(&self) -> Address {
        self.implementation_accessor().get(&self.sdk)
    }

    pub fn set_implementation(&mut self, addr: Address) {
        self.implementation_accessor().set(&mut self.sdk, addr);
    }

    pub fn is_paused(&self) -> bool {
        self.paused_accessor().get(&self.sdk)
    }

    pub fn set_paused(&mut self, value: bool) {
        self.paused_accessor().set(&mut self.sdk, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::storage_from_fixture;
    use fluentbase_sdk::{address, hex, storage::StorageDescriptor};
    use fluentbase_testing::TestingContextImpl;

    // ========================================================================
    // EIP-1967 Tests
    // ========================================================================

    #[test]
    fn test_eip1967_slot_constants() {
        // Values from https://eips.ethereum.org/EIPS/eip-1967
        assert_eq!(
            eip1967::IMPLEMENTATION,
            U256::from_be_bytes(hex!(
                "360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc"
            ))
        );
        assert_eq!(
            eip1967::ADMIN,
            U256::from_be_bytes(hex!(
                "b53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103"
            ))
        );
        assert_eq!(
            eip1967::BEACON,
            U256::from_be_bytes(hex!(
                "a3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50"
            ))
        );
    }

    #[test]
    fn test_proxy_slot_positions() {
        let sdk = TestingContextImpl::default();
        let proxy = Proxy::new(sdk);

        assert_eq!(proxy.implementation.slot(), eip1967::IMPLEMENTATION);
        assert_eq!(proxy.admin.slot(), eip1967::ADMIN);
        assert_eq!(proxy.beacon.slot(), eip1967::BEACON);
    }

    #[test]
    fn test_proxy_read_write() {
        let sdk = TestingContextImpl::default();
        let mut proxy = Proxy::new(sdk);

        let impl_addr = address!("0x1111111111111111111111111111111111111111");
        let admin_addr = address!("0x2222222222222222222222222222222222222222");
        let beacon_addr = address!("0x3333333333333333333333333333333333333333");

        proxy.set_implementation(impl_addr);
        proxy.set_admin(admin_addr);
        proxy.set_beacon(beacon_addr);

        assert_eq!(proxy.get_implementation(), impl_addr);
        assert_eq!(proxy.get_admin(), admin_addr);
        assert_eq!(proxy.get_beacon(), beacon_addr);
    }

    const PROXY_EXPECTED_LAYOUT: &str = r#"{
  "0x0000000000000000000000000000000000000000": {
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc": "0x0000000000000000000000001111111111111111111111111111111111111111",
    "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103": "0x0000000000000000000000002222222222222222222222222222222222222222",
    "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50": "0x0000000000000000000000003333333333333333333333333333333333333333"
  }
}"#;

    #[test]
    fn test_proxy_storage_layout() {
        let sdk = TestingContextImpl::default();
        let mut proxy = Proxy::new(sdk);

        proxy.set_implementation(address!("0x1111111111111111111111111111111111111111"));
        proxy.set_admin(address!("0x2222222222222222222222222222222222222222"));
        proxy.set_beacon(address!("0x3333333333333333333333333333333333333333"));

        let storage = proxy.sdk.dump_storage();
        let expected = storage_from_fixture(PROXY_EXPECTED_LAYOUT);

        assert_eq!(storage, expected);
    }

    #[test]
    fn test_proxy_slots_constant_is_zero() {
        // Explicit slots don't contribute to SLOTS count
        assert_eq!(Proxy::<TestingContextImpl>::SLOTS, 0);
    }

    // ========================================================================
    // ERC-7201 Tests
    // ========================================================================

    #[test]
    fn test_erc7201_slot_constant() {
        // Value from https://eips.ethereum.org/EIPS/eip-7201
        assert_eq!(
            erc7201::EXAMPLE_MAIN,
            U256::from_be_bytes(hex!(
                "183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500"
            ))
        );
    }

    #[test]
    fn test_erc7201_slot_alignment() {
        // ERC-7201 slots must have last byte = 0x00 (256-slot alignment)
        assert_eq!(erc7201::EXAMPLE_MAIN.byte(0), 0x00);
    }

    #[test]
    fn test_namespaced_slot_positions() {
        let sdk = TestingContextImpl::default();
        let contract = NamespacedContract::new(sdk);

        let base = erc7201::EXAMPLE_MAIN;

        // Fields laid out sequentially from namespace base
        assert_eq!(contract.data.owner.slot(), base);
        assert_eq!(contract.data.counter.slot(), base + U256::from(1));
        assert_eq!(contract.data.active.slot(), base + U256::from(2));
    }

    #[test]
    fn test_namespaced_read_write() {
        let sdk = TestingContextImpl::default();
        let mut contract = NamespacedContract::new(sdk);

        let owner = address!("0x1111111111111111111111111111111111111111");
        let counter = U256::from(42);

        contract.set_owner(owner);
        contract.set_counter(counter);
        contract.set_active(true);

        assert_eq!(contract.get_owner(), owner);
        assert_eq!(contract.get_counter(), counter);
        assert_eq!(contract.get_active(), true);
    }

    const NAMESPACED_EXPECTED_LAYOUT: &str = r#"{
  "0x0000000000000000000000000000000000000000": {
    "0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500": "0x0000000000000000000000001111111111111111111111111111111111111111",
    "0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab501": "0x000000000000000000000000000000000000000000000000000000000000002a",
    "0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab502": "0x0000000000000000000000000000000000000000000000000000000000000001"
  }
}"#;

    #[test]
    fn test_namespaced_storage_layout() {
        let sdk = TestingContextImpl::default();
        let mut contract = NamespacedContract::new(sdk);

        contract.set_owner(address!("0x1111111111111111111111111111111111111111"));
        contract.set_counter(U256::from(42));
        contract.set_active(true);

        let storage = contract.sdk.dump_storage();
        let expected = storage_from_fixture(NAMESPACED_EXPECTED_LAYOUT);

        assert_eq!(storage, expected);
    }

    #[test]
    fn test_namespaced_slots_constant_is_zero() {
        // Explicit slots don't contribute to SLOTS count
        assert_eq!(NamespacedContract::<TestingContextImpl>::SLOTS, 0);
    }

    // ========================================================================
    // Mixed Auto-Layout and Explicit Slots Tests
    // ========================================================================

    #[test]
    fn test_mixed_slot_positions() {
        let sdk = TestingContextImpl::default();
        let contract = MixedStorage::new(sdk);

        // Auto-layout fields: sequential from 0
        assert_eq!(contract.owner.slot(), U256::from(0));
        assert_eq!(contract.counter.slot(), U256::from(1));

        // Explicit slot: EIP-1967 implementation slot
        assert_eq!(contract.implementation.slot(), eip1967::IMPLEMENTATION);

        // Auto-layout continues after explicit (not affected)
        assert_eq!(contract.paused.slot(), U256::from(2));
    }

    #[test]
    fn test_mixed_slots_constant() {
        // SLOTS only counts auto-layout fields (owner, counter, paused)
        // implementation has explicit slot and is not counted
        assert_eq!(MixedStorage::<TestingContextImpl>::SLOTS, 3);
    }

    #[test]
    fn test_mixed_read_write() {
        let sdk = TestingContextImpl::default();
        let mut contract = MixedStorage::new(sdk);

        let owner = address!("0x1111111111111111111111111111111111111111");
        let impl_addr = address!("0x2222222222222222222222222222222222222222");
        let counter = U256::from(100);

        contract.set_owner(owner);
        contract.set_counter(counter);
        contract.set_implementation(impl_addr);
        contract.set_paused(true);

        assert_eq!(contract.get_owner(), owner);
        assert_eq!(contract.get_counter(), counter);
        assert_eq!(contract.get_implementation(), impl_addr);
        assert_eq!(contract.is_paused(), true);
    }

    const MIXED_EXPECTED_LAYOUT: &str = r#"{
  "0x0000000000000000000000000000000000000000": {
    "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000001111111111111111111111111111111111111111",
    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x0000000000000000000000000000000000000000000000000000000000000064",
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc": "0x0000000000000000000000002222222222222222222222222222222222222222",
    "0x0000000000000000000000000000000000000000000000000000000000000002": "0x0000000000000000000000000000000000000000000000000000000000000001"
  }
}"#;

    #[test]
    fn test_mixed_storage_layout() {
        let sdk = TestingContextImpl::default();
        let mut contract = MixedStorage::new(sdk);

        contract.set_owner(address!("0x1111111111111111111111111111111111111111"));
        contract.set_counter(U256::from(100));
        contract.set_implementation(address!("0x2222222222222222222222222222222222222222"));
        contract.set_paused(true);

        let storage = contract.sdk.dump_storage();
        let expected = storage_from_fixture(MIXED_EXPECTED_LAYOUT);

        assert_eq!(storage, expected);
    }
}
