use core::str::FromStr;
use fluentbase_sdk::{Address, U256};
use hashbrown::HashMap;
use serde_json::Value;

/// Creates storage HashMap from JSON fixture
///
/// Expected JSON format: {"contract": {"slot":"value"}}
/// ```json
/// {
///   "0x123": {
///     "0x00": "0xabc",
///     "0x01": "0xdef"
///   }
/// }
/// ```
pub fn storage_from_fixture(json: &str) -> HashMap<(Address, U256), U256> {
    let fixture: Value = serde_json::from_str(json).expect("Invalid JSON");
    let mut storage = HashMap::new();

    if let Some(contracts) = fixture.as_object() {
        for (address_str, slots) in contracts {
            let address = Address::from_str(address_str).expect("Invalid address");

            if let Some(slots_map) = slots.as_object() {
                for (slot_str, value_str) in slots_map {
                    let slot = U256::from_str(slot_str).expect("Invalid slot");
                    let value = U256::from_str(value_str.as_str().expect("Value must be string"))
                        .expect("Invalid value");

                    storage.insert((address, slot), value);
                }
            }
        }
    }

    storage
}

/// Pretty-prints storage entries grouped by contract address and sorted by slot.
pub fn format_storage(storage: &HashMap<(Address, U256), U256>) -> String {
    if storage.is_empty() {
        return "  (empty)".to_string();
    }

    let mut entries: Vec<_> = storage.iter().collect();
    entries.sort_by_key(|((addr, slot), _)| (*addr, *slot));

    let mut result = String::new();
    let mut current_addr: Option<Address> = None;

    for ((addr, slot), value) in entries {
        if current_addr != Some(*addr) {
            if current_addr.is_some() {
                result.push('\n');
            }
            result.push_str(&format!("  Contract 0x{:040x}:\n", addr));
            current_addr = Some(*addr);
        }

        result.push_str(&format!("    Slot 0x{:064x}: 0x{:064x}\n", slot, value));
    }

    result
}
