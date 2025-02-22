pub use alloy_genesis::Genesis;

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev1_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.1.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev4_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.4.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev5_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.5.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}
