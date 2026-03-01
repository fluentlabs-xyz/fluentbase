#![allow(dead_code)]
#![cfg(test)]
use fluentbase_sdk::{Address, HashMap, U256};
#[macro_export]
macro_rules! assert_storage_layout {
    // contracts with SDK type parameter
    (
        $struct_type:ident<$sdk_type:ty> => {
            $(
                $field:ident: $slot:expr, $offset:expr
            ),* $(,)?
        },
        total_slots: $total_slots:expr
    ) => {
        {
            use fluentbase_sdk::storage::StorageDescriptor;
            use fluentbase_sdk::U256;

            // Create a default SDK instance for testing
            let sdk = <$sdk_type>::default();
            let instance = $struct_type::new(sdk);

            $(
                assert_eq!(
                    instance.$field.slot(),
                    U256::from($slot),
                    "Field '{}' slot mismatch: expected {}, got {}",
                    stringify!($field),
                    $slot,
                    instance.$field.slot()
                );
                assert_eq!(
                    instance.$field.offset(),
                    $offset,
                    "Field '{}' offset mismatch: expected {}, got {}",
                    stringify!($field),
                    $offset,
                    instance.$field.offset()
                );
            )*

            assert_eq!(
                $struct_type::<$sdk_type>::SLOTS,
                $total_slots,
                "Total slots mismatch: expected {}, got {}",
                $total_slots,
                $struct_type::<$sdk_type>::SLOTS
            );
        }
    };
    // storage struct
    (
        $struct_type:ty => {
            $(
                $field:ident: $slot:expr, $offset:expr
            ),* $(,)?
        },
        total_slots: $total_slots:expr
    ) => {
        {
            use fluentbase_sdk::storage::StorageDescriptor;
            use fluentbase_sdk::U256;

            let instance = <$struct_type>::new(U256::from(0), 0);

            $(
                assert_eq!(
                    instance.$field.slot(),
                    U256::from($slot),
                    "Field '{}' slot mismatch: expected {}, got {}",
                    stringify!($field),
                    $slot,
                    instance.$field.slot()
                );
                assert_eq!(
                    instance.$field.offset(),
                    $offset,
                    "Field '{}' offset mismatch: expected {}, got {}",
                    stringify!($field),
                    $offset,
                    instance.$field.offset()
                );
            )*

            assert_eq!(
                <$struct_type>::SLOTS,
                $total_slots,
                "Total slots mismatch: expected {}, got {}",
                $total_slots,
                <$struct_type>::SLOTS
            );
        }
    };
}

/// Creates a MockStorage instance from a JSON fixture
///
/// Expected JSON format:
/// ```json
/// {
///   "expected_storage": {
///     "0x0000...0000": "0xabcd...ef12",
///     "0x0000...0001": "0x1234...5678"
///   }
/// }
/// ```
pub(crate) fn storage_from_fixture(json: &str) -> HashMap<(Address, U256), U256> {
    use core::str::FromStr;
    use serde_json::Value;

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
pub(crate) fn format_storage(storage: &HashMap<(Address, U256), U256>) -> String {
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
