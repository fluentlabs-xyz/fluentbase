//! `#[derive(Event)]` against an independent Solidity encoder: `topics[0]` and the data section
//! of an event with struct-typed fields have to be what a Solidity subscriber computes from the
//! canonical signature, with every struct expanded into its components.

use alloy_sol_types::{sol, SolEvent};
use fluentbase_sdk::{address, codec::Codec, derive::Event, Address, B256, U256};
use fluentbase_testing::TestingContextImpl;

#[derive(Codec, Clone, Debug, Default, PartialEq)]
pub struct Order {
    pub amount: U256,
    pub filled: bool,
}

/// A struct field next to an indexed value type
#[derive(Event)]
struct Filled {
    #[indexed]
    who: Address,
    order: Order,
}

/// Dynamic and fixed struct arrays
#[derive(Event)]
struct Batched {
    orders: Vec<Order>,
    pair: [Order; 2],
}

mod sol_abi {
    alloy_sol_types::sol! {
        struct Order {
            uint256 amount;
            bool filled;
        }

        event Filled(address indexed who, Order order);

        event Batched(Order[] orders, Order[2] pair);
    }
}

/// Emits a single event into a throwaway context and returns `(topics, data)`
fn emitted(emit: impl FnOnce(&mut TestingContextImpl)) -> (Vec<B256>, Vec<u8>) {
    let mut sdk = TestingContextImpl::default();
    emit(&mut sdk);
    let mut logs = sdk.take_logs();
    assert_eq!(logs.len(), 1, "expected exactly one log");
    let (data, topics) = logs.remove(0);
    (topics, data.to_vec())
}

fn order(amount: u64, filled: bool) -> Order {
    Order {
        amount: U256::from(amount),
        filled,
    }
}

/// The signature hashes the struct as `(uint256,bool)`, never as the placeholder `tuple`
#[test]
fn test_struct_field_event_signature_matches_solidity() {
    assert_eq!(Filled::SIGNATURE, "Filled(address,(uint256,bool))");
    assert_eq!(B256::new(Filled::SELECTOR), sol_abi::Filled::SIGNATURE_HASH);

    assert_eq!(
        Batched::SIGNATURE,
        "Batched((uint256,bool)[],(uint256,bool)[2])"
    );
    assert_eq!(
        B256::new(Batched::SELECTOR),
        sol_abi::Batched::SIGNATURE_HASH
    );
}

/// A canonical decoder subscribed to the Solidity event sees the emitted log
#[test]
fn test_struct_field_event_decodes_with_solidity_decoder() {
    let who = address!("1111111111111111111111111111111111111111");
    let (topics, data) = emitted(|sdk| {
        Filled {
            who,
            order: order(42, true),
        }
        .emit(sdk)
        .unwrap()
    });

    assert_eq!(topics[0], sol_abi::Filled::SIGNATURE_HASH);
    let decoded = sol_abi::Filled::decode_raw_log(&topics, &data).expect("standard decoder");
    assert_eq!(decoded.who, who);
    assert_eq!(decoded.order.amount, U256::from(42));
    assert!(decoded.order.filled);
}

#[test]
fn test_struct_array_event_decodes_with_solidity_decoder() {
    let (topics, data) = emitted(|sdk| {
        Batched {
            orders: vec![order(1, false), order(2, true), order(3, false)],
            pair: [order(4, true), order(5, false)],
        }
        .emit(sdk)
        .unwrap()
    });

    assert_eq!(topics[0], sol_abi::Batched::SIGNATURE_HASH);
    let decoded = sol_abi::Batched::decode_raw_log(&topics, &data).expect("standard decoder");
    assert_eq!(
        decoded
            .orders
            .iter()
            .map(|order| (order.amount, order.filled))
            .collect::<Vec<_>>(),
        vec![
            (U256::from(1), false),
            (U256::from(2), true),
            (U256::from(3), false)
        ]
    );
    assert_eq!(decoded.pair[0].amount, U256::from(4));
    assert!(decoded.pair[0].filled);
    assert_eq!(decoded.pair[1].amount, U256::from(5));
    assert!(!decoded.pair[1].filled);
}
