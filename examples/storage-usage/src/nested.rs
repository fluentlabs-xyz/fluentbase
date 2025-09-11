#![allow(dead_code)]
use fluentbase_sdk::{
    derive::Storage,
    storage::{
        array::StorageArray,
        composite::{Composite},
        map::StorageMap,
        primitive::StoragePrimitive,
        vec::StorageVec,
        ArrayAccess, MapAccess, PrimitiveAccess, VecAccess,
    },
    Address, SharedAPI, U256,
};

// Storage structures
#[derive(Storage)]
pub struct Item {
    owner: StoragePrimitive<Address>,
    value: StoragePrimitive<U256>,
    level: StoragePrimitive<u8>,
    active: StoragePrimitive<bool>,
}

#[derive(Storage)]
pub struct Inventory {
    equipped_items: StorageArray<Composite<Item>, 3>,
    user_items: StorageMap<Address, Composite<Item>>,
    collected_items: StorageVec<Composite<Item>>,
    total_value: StoragePrimitive<U256>,
    item_count: StoragePrimitive<u32>,
}

#[derive(Storage)]
pub struct Game<SDK> {
    sdk: SDK,
    admin: StoragePrimitive<Address>,
    version: StoragePrimitive<u32>,
    player_inventory: Composite<Inventory>,
    is_active: StoragePrimitive<bool>,
}

// Data structures for passing values
#[derive(Clone, Debug, PartialEq)]
pub struct ItemData {
    pub owner: Address,
    pub value: U256,
    pub level: u8,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryData {
    pub equipped_items: [ItemData; 3],
    pub user_items: Vec<(Address, ItemData)>,
    pub collected_items: Vec<ItemData>,
    pub total_value: U256,
    pub item_count: u32,
}

// Helper methods for Item
impl Item {
    fn set_from<SDK: SharedAPI>(&self, data: &ItemData, sdk: &mut SDK) {
        self.owner().set(sdk, data.owner);
        self.value().set(sdk, data.value);
        self.level().set(sdk, data.level);
        self.active().set(sdk, data.active);
    }

    fn get_data<SDK: SharedAPI>(&self, sdk: &SDK) -> ItemData {
        ItemData {
            owner: self.owner().get(sdk),
            value: self.value().get(sdk),
            level: self.level().get(sdk),
            active: self.active().get(sdk),
        }
    }
}

// Helper methods for Inventory
impl Inventory {
    fn set_from<SDK: SharedAPI>(&self, data: &InventoryData, sdk: &mut SDK) {
        // Set equipped items
        for (i, item_data) in data.equipped_items.iter().enumerate() {
            self.equipped_items.at(i).set_from(item_data, sdk);
        }

        // Set user items
        for (user, item_data) in &data.user_items {
            self.user_items.entry(*user).set_from(item_data, sdk);
        }

        // Set collected items
        for item_data in &data.collected_items {
            self.collected_items.push(sdk).set_from(item_data, sdk);
        }

        // Set simple fields
        self.total_value().set(sdk, data.total_value);
        self.item_count().set(sdk, data.item_count);
    }
}

// Public API methods
impl<SDK: SharedAPI> Game<SDK> {
    // Simple setters
    pub fn set_admin(&mut self, admin: Address) {
        self.admin().set(&mut self.sdk, admin);
    }

    pub fn set_version(&mut self, version: u32) {
        self.version().set(&mut self.sdk, version);
    }

    pub fn set_is_active(&mut self, active: bool) {
        self.is_active().set(&mut self.sdk, active);
    }

    // Inventory methods
    pub fn set_inventory(&mut self, data: &InventoryData) {
        self.player_inventory().set_from(data, &mut self.sdk);
    }

    pub fn set_equipped_item(&mut self, index: usize, item: &ItemData) {
        self.player_inventory()
            .equipped_items
            .at(index)
            .set_from(item, &mut self.sdk);
    }

    pub fn set_user_item(&mut self, user: Address, item: &ItemData) {
        self.player_inventory()
            .user_items
            .entry(user)
            .set_from(item, &mut self.sdk);
    }

    pub fn add_collected_item(&mut self, item: &ItemData) {
        let item_descriptor = self.player_inventory().collected_items.push(&mut self.sdk);

        item_descriptor.set_from(item, &mut self.sdk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert_storage_layout;
    use crate::nested::{Game, Inventory, Item};
    use crate::utils::storage_from_fixture;
    use fluentbase_sdk::address;
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_layout_calculations() {
        assert_storage_layout! {
            Item => {
                owner: 0, 12,
                value: 1, 0,
                level: 2, 31,
                active: 2, 30,
            },
            total_slots: 3
        }

        assert_storage_layout! {
            Inventory => {
                equipped_items: 0, 0,
                user_items: 9, 0,
                collected_items: 10, 0,
                total_value: 11, 0,
                item_count: 12, 28,
            },
            total_slots: 13
        }
        // assert_eq!(Game::<MockStorage>::REQUIRED_SLOTS, 15);
    }

    const EXPECTED_LAYOUT: &str = r#"{
  "0x0000000000000000000000000000000000000000": {
    "0x0000000000000000000000000000000000000000000000000000000000000000": "0x00000000000000000000002a1111111111111111111111111111111111111111",
    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x0000000000000000000000003333333333333333333333333333333333333333",
    "0x0000000000000000000000000000000000000000000000000000000000000002": "0x0000000000000000000000000000000000000000000000000000000000000064",
    "0x0000000000000000000000000000000000000000000000000000000000000003": "0x0000000000000000000000000000000000000000000000000000000000000101",
    "0x0000000000000000000000000000000000000000000000000000000000000004": "0x0000000000000000000000003333333333333333333333333333333333333334",
    "0x0000000000000000000000000000000000000000000000000000000000000005": "0x00000000000000000000000000000000000000000000000000000000000000c8",
    "0x0000000000000000000000000000000000000000000000000000000000000006": "0x0000000000000000000000000000000000000000000000000000000000000002",
    "0x0000000000000000000000000000000000000000000000000000000000000007": "0x0000000000000000000000003333333333333333333333333333333333333335",
    "0x0000000000000000000000000000000000000000000000000000000000000008": "0x000000000000000000000000000000000000000000000000000000000000012c",
    "0x0000000000000000000000000000000000000000000000000000000000000009": "0x0000000000000000000000000000000000000000000000000000000000000103",
    "0x000000000000000000000000000000000000000000000000000000000000000b": "0x0000000000000000000000000000000000000000000000000000000000000002",
    "0x000000000000000000000000000000000000000000000000000000000000000c": "0x0000000000000000000000000000000000000000000000000000000000002710",
    "0x000000000000000000000000000000000000000000000000000000000000000d": "0x0000000000000000000000000000000000000000000000000000000000000019",
    "0x000000000000000000000000000000000000000000000000000000000000000e": "0x0000000000000000000000000000000000000000000000000000000000000001",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01db9": "0x0000000000000000000000004444444444444444444444444444444444444444",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01dba": "0x00000000000000000000000000000000000000000000000000000000000001f4",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01dbb": "0x0000000000000000000000000000000000000000000000000000000000000105",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01dbc": "0x0000000000000000000000005555555555555555555555555555555555555555",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01dbd": "0x0000000000000000000000000000000000000000000000000000000000000258",
    "0x0175b7a638427703f0dbe7bb9bbf987a2551717b34e79f33b5b1008d1fa01dbe": "0x0000000000000000000000000000000000000000000000000000000000000006",
    "0xc92a26ffa01eee8c1ebff84f57469c156727e09aacd6f4b34ca3f2083ce10698": "0x0000000000000000000000002222222222222222222222222222222222222222",
    "0xc92a26ffa01eee8c1ebff84f57469c156727e09aacd6f4b34ca3f2083ce10699": "0x00000000000000000000000000000000000000000000000000000000000003e7",
    "0xc92a26ffa01eee8c1ebff84f57469c156727e09aacd6f4b34ca3f2083ce1069a": "0x000000000000000000000000000000000000000000000000000000000000010a"
  }
}
"#;

    #[test]
    fn test_storage_layout_with_data_structures() {
        let sdk = HostTestingContext::default();

        let mut game = Game::new(sdk, U256::from(0), 0);

        // Set simple fields
        game.set_admin(address!("0x1111111111111111111111111111111111111111"));
        game.set_version(42);
        game.set_is_active(true);

        let inventory_data = InventoryData {
            equipped_items: [
                ItemData {
                    owner: address!("0x3333333333333333333333333333333333333333"),
                    value: U256::from(100),
                    level: 1,
                    active: true,
                },
                ItemData {
                    owner: address!("0x3333333333333333333333333333333333333334"),
                    value: U256::from(200),
                    level: 2,
                    active: false,
                },
                ItemData {
                    owner: address!("0x3333333333333333333333333333333333333335"),
                    value: U256::from(300),
                    level: 3,
                    active: true,
                },
            ],
            user_items: vec![(
                address!("0x2222222222222222222222222222222222222222"),
                ItemData {
                    owner: address!("0x2222222222222222222222222222222222222222"),
                    value: U256::from(999),
                    level: 10,
                    active: true,
                },
            )],
            collected_items: vec![
                ItemData {
                    owner: address!("0x4444444444444444444444444444444444444444"),
                    value: U256::from(500),
                    level: 5,
                    active: true,
                },
                ItemData {
                    owner: address!("0x5555555555555555555555555555555555555555"),
                    value: U256::from(600),
                    level: 6,
                    active: false,
                },
            ],
            total_value: U256::from(10000),
            item_count: 25,
        };
        game.set_inventory(&inventory_data);
        let storage = game.sdk.dump_storage();

        // print resulting storage
        // println!("{}", format_storage(&storage));

        let expected_storage = storage_from_fixture(EXPECTED_LAYOUT);
        assert_eq!(expected_storage, storage);
    }
}
