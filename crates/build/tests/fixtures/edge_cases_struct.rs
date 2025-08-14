// Test case: Edge cases and boundary conditions

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Codec},
    Address,
    SharedAPI,
    B256,
    U256,
};

// Edge case: Empty struct
#[derive(Codec, Debug, Clone)]
pub struct Empty {}

// Edge case: Single field struct
#[derive(Codec, Debug, Clone)]
pub struct Single {
    pub value: U256,
}

// Edge case: Tuple struct (unnamed fields)
#[derive(Codec, Debug, Clone)]
pub struct TupleStruct(pub Address, pub U256, pub bool);

// Edge case: Unit struct
#[derive(Codec, Debug, Clone)]
pub struct UnitStruct;

// Edge case: Large struct (many fields)
#[derive(Codec, Debug, Clone)]
pub struct LargeStruct {
    pub field1: U256,
    pub field2: Address,
    pub field3: bool,
    pub field4: u8,
    pub field5: u16,
    pub field6: u32,
    pub field7: u64,
    pub field8: B256,
    pub field9: String,
    pub field10: U256,
    pub field11: Address,
    pub field12: bool,
}

// Edge case: Struct with all primitive types
#[derive(Codec, Debug, Clone)]
pub struct AllTypes {
    pub address_field: Address,
    pub uint256_field: U256,
    pub bool_field: bool,
    pub bytes32_field: B256,
    pub string_field: String,
    pub uint8_field: u8,
    pub uint16_field: u16,
    pub uint32_field: u32,
    pub uint64_field: u64,
}

// Edge case: Deeply nested same type
#[derive(Codec, Debug, Clone)]
pub struct Level1 {
    pub value: U256,
}

#[derive(Codec, Debug, Clone)]
pub struct Level2 {
    pub inner: Level1,
}

#[derive(Codec, Debug, Clone)]
pub struct Level3 {
    pub inner: Level2,
}

#[derive(Codec, Debug, Clone)]
pub struct Level4 {
    pub inner: Level3,
}

#[derive(Codec, Debug, Clone)]
pub struct Level5 {
    pub inner: Level4,
}

// Note: Struct without Codec for negative testing
pub struct NoCodec {
    pub value: U256,
}

#[derive(Default)]
pub struct EdgeCasesContract<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> EdgeCasesContract<SDK> {
    /// Test: empty struct
    pub fn use_empty(&mut self, empty: Empty) -> bool {
        true
    }

    /// Test: single field struct
    pub fn use_single(&mut self, single: Single) -> U256 {
        single.value
    }

    /// Test: tuple struct (unnamed fields)
    pub fn use_tuple_struct(&mut self, ts: TupleStruct) -> Address {
        ts.0
    }

    /// Test: unit struct
    pub fn use_unit(&mut self, _unit: UnitStruct) -> bool {
        true
    }

    /// Test: large struct with many fields
    pub fn use_large(&mut self, large: LargeStruct) -> bool {
        large.field3
    }

    /// Test: all primitive types
    pub fn use_all_types(&mut self, all: AllTypes) -> Address {
        all.address_field
    }

    /// Test: deeply nested (5 levels)
    pub fn use_deeply_nested(&mut self, nested: Level5) -> U256 {
        nested.inner.inner.inner.inner.value
    }

    /// Test: return empty struct
    pub fn get_empty(&self) -> Empty {
        Empty {}
    }

    /// Test: return tuple struct
    pub fn get_tuple_struct(&self) -> TupleStruct {
        TupleStruct(Address::from([1u8; 20]), U256::from(42), true)
    }

    /// Test: mixed - some structs, some primitives
    pub fn mixed_params(
        &mut self,
        single: Single,
        addr: Address,
        empty: Empty,
        value: U256,
    ) -> bool {
        single.value == value
    }
}

impl<SDK: SharedAPI> EdgeCasesContract<SDK> {
    pub fn new(sdk: SDK) -> Self {
        Self { sdk }
    }
}

basic_entrypoint!(EdgeCasesContract);
