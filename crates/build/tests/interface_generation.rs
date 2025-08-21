// Solidity interface generation tests from ABI using insta snapshots

use fluentbase_build::solidity::generate_interface;
use insta::assert_snapshot;
use serde_json::json;

#[test]
fn interface_with_comprehensive_structs() {
    // Comprehensive test case: combines multiple structs, arrays, and nested structures
    let abi = vec![
        json!({
            "name": "calculateSlippage",
            "type": "function",
            "inputs": [
                {
                    "name": "params",
                    "type": "tuple",
                    "internalType": "struct SlippageParams",
                    "components": [
                        {"name": "amount_in", "type": "uint256", "internalType": "uint256"},
                        {"name": "reserve_in", "type": "uint256", "internalType": "uint256"},
                        {"name": "reserve_out", "type": "uint256", "internalType": "uint256"},
                        {"name": "fee_rate", "type": "uint256", "internalType": "uint256"}
                    ]
                }
            ],
            "outputs": [
                {"name": "_0", "type": "uint256", "internalType": "uint256"}
            ],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "optimizeSwap",
            "type": "function",
            "inputs": [
                {"name": "total_amount", "type": "uint256", "internalType": "uint256"},
                {"name": "reserve_in", "type": "uint256", "internalType": "uint256"},
                {"name": "reserve_out", "type": "uint256", "internalType": "uint256"}
            ],
            "outputs": [
                {
                    "name": "_0",
                    "type": "tuple",
                    "internalType": "struct OptimizationResult",
                    "components": [
                        {"name": "optimal_amount", "type": "uint256", "internalType": "uint256"},
                        {"name": "expected_output", "type": "uint256", "internalType": "uint256"},
                        {"name": "price_impact", "type": "uint256", "internalType": "uint256"}
                    ]
                }
            ],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "findOptimalRoute",
            "type": "function",
            "inputs": [
                {"name": "amount_in", "type": "uint256", "internalType": "uint256"},
                {
                    "name": "pools",
                    "type": "tuple[]",
                    "internalType": "struct Pool[]",
                    "components": [
                        {"name": "reserve_in", "type": "uint256", "internalType": "uint256"},
                        {"name": "reserve_out", "type": "uint256", "internalType": "uint256"}
                    ]
                },
                {"name": "fee_rates", "type": "uint256[]", "internalType": "uint256[]"}
            ],
            "outputs": [
                {"name": "_0", "type": "uint256", "internalType": "uint256"}
            ],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "executeRoute",
            "type": "function",
            "inputs": [
                {
                    "name": "route",
                    "type": "tuple",
                    "internalType": "struct Route",
                    "components": [
                        {
                            "name": "pools",
                            "type": "tuple[]",
                            "internalType": "struct Pool[]",
                            "components": [
                                {"name": "reserve_in", "type": "uint256", "internalType": "uint256"},
                                {"name": "reserve_out", "type": "uint256", "internalType": "uint256"}
                            ]
                        },
                        {"name": "tokens", "type": "address[]", "internalType": "address[]"},
                        {"name": "input_amount", "type": "uint256", "internalType": "uint256"}
                    ]
                }
            ],
            "outputs": [
                {"name": "_0", "type": "uint256", "internalType": "uint256"}
            ],
            "stateMutability": "nonpayable"
        }),
    ];

    let interface = generate_interface("ComprehensiveContract", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_deeply_nested_structs() {
    // Test case: deep nesting with 4+ levels
    let abi = vec![json!({
        "name": "processComplexData",
        "type": "function",
        "inputs": [
            {
                "name": "data",
                "type": "tuple",
                "internalType": "struct ComplexData",
                "components": [
                    {
                        "name": "id",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "batch",
                        "type": "tuple",
                        "internalType": "struct Batch",
                        "components": [
                            {
                                "name": "timestamp",
                                "type": "uint256",
                                "internalType": "uint256"
                            },
                            {
                                "name": "orders",
                                "type": "tuple[]",
                                "internalType": "struct Order[]",
                                "components": [
                                    {
                                        "name": "orderId",
                                        "type": "uint256",
                                        "internalType": "uint256"
                                    },
                                    {
                                        "name": "user",
                                        "type": "tuple",
                                        "internalType": "struct User",
                                        "components": [
                                            {
                                                "name": "addr",
                                                "type": "address",
                                                "internalType": "address"
                                            },
                                            {
                                                "name": "profile",
                                                "type": "tuple",
                                                "internalType": "struct Profile",
                                                "components": [
                                                    {
                                                        "name": "name",
                                                        "type": "string",
                                                        "internalType": "string"
                                                    },
                                                    {
                                                        "name": "level",
                                                        "type": "uint256",
                                                        "internalType": "uint256"
                                                    }
                                                ]
                                            }
                                        ]
                                    },
                                    {
                                        "name": "items",
                                        "type": "tuple[]",
                                        "internalType": "struct Item[]",
                                        "components": [
                                            {
                                                "name": "itemId",
                                                "type": "uint256",
                                                "internalType": "uint256"
                                            },
                                            {
                                                "name": "quantity",
                                                "type": "uint256",
                                                "internalType": "uint256"
                                            }
                                        ]
                                    }
                                ]
                            }
                        ]
                    }
                ]
            }
        ],
        "outputs": [
            {"name": "success", "type": "bool", "internalType": "bool"}
        ],
        "stateMutability": "nonpayable"
    })];

    let interface = generate_interface("DeepNestingContract", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_multidimensional_arrays() {
    // Test case: 2D and 3D arrays of structs
    let abi = vec![
        json!({
            "name": "process2DMatrix",
            "type": "function",
            "inputs": [
                {
                    "name": "matrix",
                    "type": "tuple[][]",
                    "internalType": "struct Cell[][]",
                    "components": [
                        {"name": "value", "type": "uint256", "internalType": "uint256"},
                        {"name": "metadata", "type": "bytes32", "internalType": "bytes32"}
                    ]
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "process3DMatrix",
            "type": "function",
            "inputs": [
                {
                    "name": "cube",
                    "type": "tuple[][][]",
                    "internalType": "struct Point[][][]",
                    "components": [
                        {"name": "x", "type": "uint256", "internalType": "uint256"},
                        {"name": "y", "type": "uint256", "internalType": "uint256"},
                        {"name": "z", "type": "uint256", "internalType": "uint256"}
                    ]
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
    ];

    let interface = generate_interface("MultidimensionalArrayContract", &abi).unwrap();
    assert_snapshot!(interface);
}
#[test]
fn interface_with_anonymous_tuple_arrays() {
    let abi = vec![json!({
        "name": "processData",
        "type": "function",
        "inputs": [
            {
                "name": "simpleData",
                "type": "tuple",
                "components": [
                    {"name": "a", "type": "uint256", "internalType": "uint256"},
                    {"name": "b", "type": "address", "internalType": "address"}
                ]
            },
            {
                "name": "arrayData",
                "type": "tuple[]",
                "components": [
                    {"name": "x", "type": "uint256", "internalType": "uint256"},
                    {"name": "y", "type": "bool", "internalType": "bool"}
                ]
            },
            {
                "name": "matrixData",
                "type": "uint256[][]",
                "internalType": "uint256[][]"
            },
            {
                "name": "complexTupleMatrix",
                "type": "tuple[][]",
                "components": [
                    {"name": "x", "type": "uint256", "internalType": "uint256"},
                    {"name": "y", "type": "uint256", "internalType": "uint256"},
                    {"name": "z", "type": "bool", "internalType": "bool"}
                ]
            }
        ],
        "outputs": [],
        "stateMutability": "nonpayable"
    })];

    let interface = generate_interface("AnonymousTuples", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_fixed_size_arrays() {
    // Test case: fixed-size arrays and multidimensional fixed arrays
    let abi = vec![json!({
        "name": "processFixedArrays",
        "type": "function",
        "inputs": [
            {
                "name": "data",
                "type": "tuple",
                "internalType": "struct FixedArrayData",
                "components": [
                    {"name": "addresses", "type": "address[3]", "internalType": "address[3]"},
                    {"name": "values", "type": "uint256[5]", "internalType": "uint256[5]"},
                    {"name": "matrix", "type": "uint256[3][2]", "internalType": "uint256[3][2]"},
                    {"name": "flags", "type": "bool[2]", "internalType": "bool[2]"}
                ]
            },
            {
                "name": "fixedStructArray",
                "type": "tuple[3]",
                "internalType": "struct Item[3]",
                "components": [
                    {"name": "id", "type": "uint256", "internalType": "uint256"},
                    {"name": "data", "type": "bytes32", "internalType": "bytes32"}
                ]
            }
        ],
        "outputs": [],
        "stateMutability": "nonpayable"
    })];

    let interface = generate_interface("FixedArrayContract", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_bytes_and_string_types() {
    // Test case: various bytes types and strings
    let abi = vec![json!({
        "name": "processMessage",
        "type": "function",
        "inputs": [
            {
                "name": "msg",
                "type": "tuple",
                "internalType": "struct Message",
                "components": [
                    {"name": "content", "type": "string", "internalType": "string"},
                    {"name": "signature", "type": "bytes", "internalType": "bytes"},
                    {"name": "hash", "type": "bytes32", "internalType": "bytes32"},
                    {"name": "shortData", "type": "bytes4", "internalType": "bytes4"},
                    {"name": "mediumData", "type": "bytes16", "internalType": "bytes16"}
                ]
            }
        ],
        "outputs": [
            {"name": "verified", "type": "bool", "internalType": "bool"}
        ],
        "stateMutability": "pure"
    })];

    let interface = generate_interface("MessageProcessor", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_edge_cases() {
    // Test case: empty struct and single-field struct
    let abi = vec![
        json!({
            "name": "processEmpty",
            "type": "function",
            "inputs": [
                {
                    "name": "empty",
                    "type": "tuple",
                    "internalType": "struct Empty",
                    "components": []
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "processSingle",
            "type": "function",
            "inputs": [
                {
                    "name": "single",
                    "type": "tuple",
                    "internalType": "struct Single",
                    "components": [
                        {"name": "value", "type": "uint256", "internalType": "uint256"}
                    ]
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
    ];

    let interface = generate_interface("EdgeCasesContract", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_struct_deduplication() {
    // Test case: same struct used in multiple functions (should only define once)
    let abi = vec![
        json!({
            "name": "setUser",
            "type": "function",
            "inputs": [
                {
                    "name": "user",
                    "type": "tuple",
                    "internalType": "struct User",
                    "components": [
                        {"name": "id", "type": "uint256", "internalType": "uint256"},
                        {"name": "addr", "type": "address", "internalType": "address"}
                    ]
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
        json!({
            "name": "getUser",
            "type": "function",
            "inputs": [
                {"name": "id", "type": "uint256", "internalType": "uint256"}
            ],
            "outputs": [
                {
                    "name": "user",
                    "type": "tuple",
                    "internalType": "struct User",
                    "components": [
                        {"name": "id", "type": "uint256", "internalType": "uint256"},
                        {"name": "addr", "type": "address", "internalType": "address"}
                    ]
                }
            ],
            "stateMutability": "view"
        }),
        json!({
            "name": "updateUsers",
            "type": "function",
            "inputs": [
                {
                    "name": "users",
                    "type": "tuple[]",
                    "internalType": "struct User[]",
                    "components": [
                        {"name": "id", "type": "uint256", "internalType": "uint256"},
                        {"name": "addr", "type": "address", "internalType": "address"}
                    ]
                }
            ],
            "outputs": [],
            "stateMutability": "nonpayable"
        }),
    ];

    let interface = generate_interface("UserManager", &abi).unwrap();
    assert_snapshot!(interface);
}

#[test]
fn interface_with_all_solidity_types() {
    // Test case: struct with all Solidity primitive types
    let abi = vec![json!({
        "name": "processAllTypes",
        "type": "function",
        "inputs": [
            {
                "name": "data",
                "type": "tuple",
                "internalType": "struct AllTypes",
                "components": [
                    {"name": "u8Val", "type": "uint8", "internalType": "uint8"},
                    {"name": "u16Val", "type": "uint16", "internalType": "uint16"},
                    {"name": "u32Val", "type": "uint32", "internalType": "uint32"},
                    {"name": "u64Val", "type": "uint64", "internalType": "uint64"},
                    {"name": "u128Val", "type": "uint128", "internalType": "uint128"},
                    {"name": "u256Val", "type": "uint256", "internalType": "uint256"},
                    {"name": "i8Val", "type": "int8", "internalType": "int8"},
                    {"name": "i16Val", "type": "int16", "internalType": "int16"},
                    {"name": "i32Val", "type": "int32", "internalType": "int32"},
                    {"name": "i64Val", "type": "int64", "internalType": "int64"},
                    {"name": "i128Val", "type": "int128", "internalType": "int128"},
                    {"name": "i256Val", "type": "int256", "internalType": "int256"},
                    {"name": "boolVal", "type": "bool", "internalType": "bool"},
                    {"name": "addrVal", "type": "address", "internalType": "address"},
                    {"name": "b1Val", "type": "bytes1", "internalType": "bytes1"},
                    {"name": "b16Val", "type": "bytes16", "internalType": "bytes16"},
                    {"name": "b32Val", "type": "bytes32", "internalType": "bytes32"},
                    {"name": "strVal", "type": "string", "internalType": "string"},
                    {"name": "bytesVal", "type": "bytes", "internalType": "bytes"}
                ]
            }
        ],
        "outputs": [],
        "stateMutability": "nonpayable"
    })];

    let interface = generate_interface("AllTypesContract", &abi).unwrap();
    assert_snapshot!(interface);
}
