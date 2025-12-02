use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{Address, B256, U256};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;
use revm::Database;

#[test]
fn test_block_hash() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let block_hash_0: B256 = ctx.db.block_hash(0).unwrap();
    let get_block_hash_selector = hex!("ee82ac5e");

    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        hex::decode(include_bytes!("../assets/BlockHash.bin"))
            .unwrap()
            .into(),
    );

    // Case 1: diff = 0 (current block) - should return B256::ZERO
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        [
            get_block_hash_selector.as_slice(),
            &U256::from(0).to_be_bytes::<32>(),
        ]
        .concat()
        .into(),
        None,
        None,
    );
    assert!(result.is_success());
    let output = result.output().unwrap();
    assert_eq!(
        output.as_ref(),
        &B256::ZERO.0,
        "blockhash() should return zero for current block (diff = 0)"
    );

    // Case 2: Move to block 100 and read block 0 - should return valid value
    ctx = ctx.with_block_number(100);
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        [
            get_block_hash_selector.as_slice(),
            &U256::from(0).to_be_bytes::<32>(),
        ]
        .concat()
        .into(),
        None,
        None,
    );
    assert!(result.is_success());
    let output = result.output().unwrap();
    assert_eq!(
        output.as_ref(),
        &block_hash_0.0,
        "blockhash(0) should return valid hash when diff <= 256 (current block = 100)"
    );

    // Case 3: Move beyond 256 blocks and read block 0 - should return zero
    ctx = ctx.with_block_number(300);
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        [
            get_block_hash_selector.as_slice(),
            &U256::from(0).to_be_bytes::<32>(),
        ]
        .concat()
        .into(),
        None,
        None,
    );
    assert!(result.is_success());
    let output = result.output().unwrap();
    assert_eq!(
        output.as_ref(),
        &B256::ZERO.0,
        "blockhash(0) should return zero when diff > 256 (current block = 300)"
    );
}
