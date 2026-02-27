#![allow(clippy::assign_op_pattern)]
#![allow(dead_code)]
use fluentbase_sdk::{
    derive::Contract,
    storage::{
        StorageAddress, StorageBool, StorageMap, StorageString, StorageU256, StorageU32, StorageVec,
    },
    Address, SharedAPI, U256,
};

#[derive(Contract)]
pub struct App<SDK> {
    sdk: SDK,
    owner: StorageAddress,
    counter: StorageU256,
    balances: StorageMap<Address, StorageMap<Address, StorageU256>>,
    data: StorageVec<StorageU256>,
    name: StorageString,
    description: StorageString,
    is_active: StorageBool,
    is_paused: StorageBool,
    is_locked: StorageBool,
    version: StorageU32,
    flags: StorageU32,
}

// Data structures for passing values
#[derive(Clone, Debug, PartialEq)]
pub struct StateData {
    pub owner: Address,
    pub counter: U256,
    pub balances: Vec<(Address, Address, U256)>, // (owner, token, amount)
    pub data_elements: Vec<U256>,
    pub name: String,
    pub description: String,
    pub is_active: bool,
    pub is_paused: bool,
    pub is_locked: bool,
    pub version: u32,
    pub flags: u32,
}

// Public API methods
impl<SDK: SharedAPI> App<SDK> {
    pub fn set_state(&mut self, data: &StateData) {
        self.owner_accessor().set(&mut self.sdk, data.owner);
        self.counter_accessor().set(&mut self.sdk, data.counter);

        // Set balances
        for (owner, token, amount) in &data.balances {
            self.balances_accessor()
                .entry(*owner)
                .entry(*token)
                .set(&mut self.sdk, *amount);
        }

        // Set vector elements
        for element in &data.data_elements {
            self.data_accessor().push(&mut self.sdk, *element);
        }

        // Set strings
        self.name_accessor().set(&mut self.sdk, &data.name);
        self.description_accessor()
            .set(&mut self.sdk, &data.description);

        // Set packed fields
        self.is_active_accessor().set(&mut self.sdk, data.is_active);
        self.is_paused_accessor().set(&mut self.sdk, data.is_paused);
        self.is_locked_accessor().set(&mut self.sdk, data.is_locked);
        self.version_accessor().set(&mut self.sdk, data.version);
        self.flags_accessor().set(&mut self.sdk, data.flags);
    }

    pub fn get_state(&self) -> StateData {
        // Get vector elements
        let mut data_elements = Vec::new();
        let vec_len = self.data_accessor().len(&self.sdk);
        for i in 0..vec_len {
            data_elements.push(self.data_accessor().at(i).get(&self.sdk));
        }

        StateData {
            owner: self.owner_accessor().get(&self.sdk),
            counter: self.counter_accessor().get(&self.sdk),
            balances: vec![], // Would need to track which keys were set
            data_elements,
            name: self.name_accessor().get(&self.sdk),
            description: self.description_accessor().get(&self.sdk),
            is_active: self.is_active_accessor().get(&self.sdk),
            is_paused: self.is_paused_accessor().get(&self.sdk),
            is_locked: self.is_locked_accessor().get(&self.sdk),
            version: self.version_accessor().get(&self.sdk),
            flags: self.flags_accessor().get(&self.sdk),
        }
    }

    // Individual setters for convenience
    pub fn set_owner(&mut self, owner: Address) {
        self.owner_accessor().set(&mut self.sdk, owner);
    }

    pub fn set_counter(&mut self, counter: U256) {
        self.counter_accessor().set(&mut self.sdk, counter);
    }

    pub fn set_balance(&mut self, owner: Address, token: Address, amount: U256) {
        self.balances_accessor()
            .entry(owner)
            .entry(token)
            .set(&mut self.sdk, amount);
    }

    pub fn get_balance(&self, owner: Address, token: Address) -> U256 {
        self.balances_accessor()
            .entry(owner)
            .entry(token)
            .get(&self.sdk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{assert_storage_layout, utils::storage_from_fixture};
    use fluentbase_sdk::address;
    use fluentbase_testing::TestingContextImpl;

    #[test]
    fn test_layout_calculations() {
        assert_storage_layout! {
            App<TestingContextImpl> => {
                owner: 0, 12,
                counter: 1, 0,
                balances: 2, 0,
                data: 3, 0,
                name: 4, 0,
                description: 5, 0,
                is_active: 6, 31,
                is_paused: 6, 30,
                is_locked: 6, 29,
                version: 6, 25,
                flags: 6, 21,
            },
            total_slots: 7
        }
    }

    const EXPECTED_LAYOUT: &str = r#"{
  "0x0000000000000000000000000000000000000000": {
    "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000001111111111111111111111111111111111111111",
    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x000000000000000000000000000000000000000000000000000000000000002a",
    "0x0000000000000000000000000000000000000000000000000000000000000003": "0x0000000000000000000000000000000000000000000000000000000000000003",
    "0x0000000000000000000000000000000000000000000000000000000000000004": "0x7465737400000000000000000000000000000000000000000000000000000008",
    "0x0000000000000000000000000000000000000000000000000000000000000005": "0x00000000000000000000000000000000000000000000000000000000000000b3",
    "0x0000000000000000000000000000000000000000000000000000000000000006": "0x000000000000000000000000000000000000000000deadbeef00003039010001",
    "0x036b6384b5eca791c62761152d0c79bb0604c104a5fb6f4eb0703f3154bb3db0": "0x7468697320697320612076657279206c6f6e67206465736372697074696f6e20",
    "0x036b6384b5eca791c62761152d0c79bb0604c104a5fb6f4eb0703f3154bb3db1": "0x74686174206578636565647320333120627974657320616e642073686f756c64",
    "0x036b6384b5eca791c62761152d0c79bb0604c104a5fb6f4eb0703f3154bb3db2": "0x2062652073746f726564206173206c6f6e6720737472696e6700000000000000",
    "0xb34395caba8110d7cdff9a58077ab0a87eb0ab00b5e8dff5b3370b0e6a6b7744": "0x00000000000000000000000000000000000000000000000000000000000003e8",
    "0xc2575a0e9e593c00f959f8c92f12db2869c3395a3b0502d05e2516446f71f85b": "0x000000000000000000000000000000000000000000000000000000000000006f",
    "0xc2575a0e9e593c00f959f8c92f12db2869c3395a3b0502d05e2516446f71f85c": "0x00000000000000000000000000000000000000000000000000000000000000de",
    "0xc2575a0e9e593c00f959f8c92f12db2869c3395a3b0502d05e2516446f71f85d": "0x000000000000000000000000000000000000000000000000000000000000014d"
    }
}"#;

    #[test]
    fn test_storage_layout_with_data() {
        let sdk = TestingContextImpl::default();
        let mut app = App::new(sdk);

        let state_data = StateData {
            owner: address!("0x1111111111111111111111111111111111111111"),
            counter: U256::from(42),
            balances: vec![(
                address!("0x2222222222222222222222222222222222222222"),
                address!("0x3333333333333333333333333333333333333333"),
                U256::from(1000),
            )],
            data_elements: vec![U256::from(111), U256::from(222), U256::from(333)],
            name: "test".to_string(),
            description: "this is a very long description that exceeds 31 bytes and should be stored as long string".to_string(),
            is_active: true,
            is_paused: false,
            is_locked: true,
            version: 12345,
            flags: 0xDEADBEEF,
        };

        // Set all state at once
        app.set_state(&state_data);

        // Verify we can read it back correctly
        let retrieved = app.get_state();
        assert_eq!(retrieved.owner, state_data.owner);
        assert_eq!(retrieved.counter, state_data.counter);
        assert_eq!(retrieved.data_elements, state_data.data_elements);
        assert_eq!(retrieved.name, state_data.name);
        assert_eq!(retrieved.description, state_data.description);
        assert_eq!(retrieved.is_active, state_data.is_active);
        assert_eq!(retrieved.is_paused, state_data.is_paused);
        assert_eq!(retrieved.is_locked, state_data.is_locked);
        assert_eq!(retrieved.version, state_data.version);
        assert_eq!(retrieved.flags, state_data.flags);

        // Verify balance through direct getter
        assert_eq!(
            app.get_balance(
                address!("0x2222222222222222222222222222222222222222"),
                address!("0x3333333333333333333333333333333333333333")
            ),
            U256::from(1000)
        );

        // Dump and compare storage
        let storage = app.sdk.dump_storage();

        let expected_storage = storage_from_fixture(EXPECTED_LAYOUT);
        assert_eq!(expected_storage, storage);
    }
}
