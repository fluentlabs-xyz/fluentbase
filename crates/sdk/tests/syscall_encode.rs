use fluentbase_sdk::{syscall::encode, Address, B256, U256};

fn address() -> Address {
    Address::from([0x11; 20])
}

fn slot() -> U256 {
    U256::from(0x22u64)
}

fn value() -> U256 {
    U256::from(0x33u64)
}

#[test]
fn encodes_storage_and_metadata_inputs() {
    let address = address();
    let slot = slot();
    let value = value();

    let mut output = Vec::new();
    encode::storage_read_into(&mut output, &slot);
    assert_eq!(output.len(), encode::storage_read_size_hint());
    assert_eq!(output, slot.as_le_slice());

    output.clear();
    encode::storage_write_into(&mut output, &slot, &value);
    assert_eq!(output.len(), encode::storage_write_size_hint());
    assert_eq!(&output[..32], slot.as_le_slice());
    assert_eq!(&output[32..], value.as_le_slice());

    output.clear();
    encode::metadata_size_into(&mut output, &address);
    assert_eq!(output.len(), encode::metadata_size_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::metadata_account_owner_into(&mut output, &address);
    assert_eq!(output.len(), encode::metadata_account_owner_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::metadata_copy_into(&mut output, &address, 0x4455_6677, 0x8899_aabb);
    assert_eq!(output.len(), encode::metadata_copy_size_hint());
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..24], &0x4455_6677u32.to_le_bytes());
    assert_eq!(&output[24..], &0x8899_aabbu32.to_le_bytes());

    output.clear();
    encode::metadata_storage_read_into(&mut output, &slot);
    assert_eq!(output.len(), encode::metadata_storage_read_size_hint());
    assert_eq!(output, slot.as_le_slice());

    output.clear();
    encode::metadata_storage_write_into(&mut output, &slot, &value);
    assert_eq!(output.len(), encode::metadata_storage_write_size_hint());
    assert_eq!(&output[..32], slot.as_le_slice());
    assert_eq!(&output[32..], value.as_le_slice());

    let metadata = [0xde, 0xad, 0xbe, 0xef];
    output.clear();
    encode::metadata_write_into(&mut output, &address, 7, metadata);
    assert_eq!(
        output.len(),
        encode::metadata_write_size_hint(metadata.len())
    );
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..24], &7u32.to_le_bytes());
    assert_eq!(&output[24..], &metadata);

    output.clear();
    encode::metadata_create_into(&mut output, &slot, &metadata);
    assert_eq!(
        output.len(),
        encode::metadata_create_size_hint(metadata.len())
    );
    assert_eq!(&output[..32], &slot.to_be_bytes::<32>());
    assert_eq!(&output[32..], &metadata);
}

#[test]
fn encodes_transient_storage_and_logs() {
    let slot = slot();
    let value = value();
    let mut output = Vec::new();

    encode::transient_write_into(&mut output, &slot, &value);
    assert_eq!(output.len(), encode::transient_write_size_hint());
    assert_eq!(&output[..32], slot.as_le_slice());
    assert_eq!(&output[32..], value.as_le_slice());

    output.clear();
    encode::transient_write_into(&mut output, &U256::ZERO, &U256::ZERO);
    assert_eq!(output, [0u8; 64]);

    output.clear();
    encode::transient_read_into(&mut output, &slot);
    assert_eq!(output.len(), encode::transient_read_size_hint());
    assert_eq!(output, slot.as_le_slice());

    let topics = [B256::from([0x44; 32]), B256::from([0x55; 32])];
    let data = [0x66, 0x77, 0x88];
    output.clear();
    encode::emit_log_into(&mut output, &topics, &data);
    assert_eq!(
        output.len(),
        encode::emit_log_size_hint(topics.len(), data.len())
    );
    assert_eq!(output[0], topics.len() as u8);
    assert_eq!(&output[1..33], topics[0].as_slice());
    assert_eq!(&output[33..65], topics[1].as_slice());
    assert_eq!(&output[65..], &data);
}

#[test]
fn encodes_balance_block_and_code_inputs() {
    let address = address();
    let mut output = Vec::new();

    encode::self_balance_into(&mut output);
    assert_eq!(output.len(), encode::self_balance_size_hint());

    encode::balance_into(&mut output, &address);
    assert_eq!(output.len(), encode::balance_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::block_hash_into(&mut output, 0x1122_3344_5566_7788);
    assert_eq!(output.len(), encode::block_hash_size_hint());
    assert_eq!(output, 0x1122_3344_5566_7788u64.to_le_bytes());

    output.clear();
    encode::code_size_into(&mut output, &address);
    assert_eq!(output.len(), encode::code_size_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::code_hash_into(&mut output, &address);
    assert_eq!(output.len(), encode::code_hash_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::code_copy_into(&mut output, &address, 9, 11);
    assert_eq!(output.len(), encode::code_copy_size_hint());
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..28], &9u64.to_le_bytes());
    assert_eq!(&output[28..], &11u64.to_le_bytes());
}

#[test]
fn encodes_calls_account_destruction_and_runtime_upgrades() {
    let address = address();
    let salt = slot();
    let value = value();
    let input = [0xaa, 0xbb, 0xcc];
    let mut output = Vec::new();

    encode::create_into(&mut output, None, &value, &input);
    assert_eq!(output.len(), encode::create_size_hint(input.len(), false));
    assert_eq!(&output[..32], value.as_le_slice());
    assert_eq!(&output[32..], &input);

    output.clear();
    encode::create_into(&mut output, Some(&salt), &value, &input);
    assert_eq!(output.len(), encode::create_size_hint(input.len(), true));
    assert_eq!(&output[..32], value.as_le_slice());
    assert_eq!(&output[32..64], salt.as_le_slice());
    assert_eq!(&output[64..], &input);

    output.clear();
    encode::call_into(&mut output, address, None, &input);
    assert_eq!(output.len(), encode::call_size_hint(input.len(), false));
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..], &input);

    output.clear();
    encode::call_into(&mut output, address, Some(value), &input);
    assert_eq!(output.len(), encode::call_size_hint(input.len(), true));
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..52], value.as_le_slice());
    assert_eq!(&output[52..], &input);

    output.clear();
    encode::delegate_call_into(&mut output, &address, &input);
    assert_eq!(output.len(), encode::delegate_call_size_hint(input.len()));
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..], &input);

    output.clear();
    encode::static_call_into(&mut output, &address, &input);
    assert_eq!(output.len(), encode::static_call_size_hint(input.len()));
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..], &input);

    output.clear();
    encode::destroy_account_into(&mut output, &address);
    assert_eq!(output.len(), encode::destroy_account_size_hint());
    assert_eq!(output, address.as_slice());

    output.clear();
    encode::upgrade_runtime_into(&mut output, &address, &input);
    assert_eq!(output.len(), encode::upgrade_runtime_size_hint(input.len()));
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..], &input);

    output.clear();
    encode::upgrade_evm_runtime_into(&mut output, &address, &input);
    assert_eq!(
        output.len(),
        encode::upgrade_evm_runtime_size_hint(input.len())
    );
    assert_eq!(&output[..20], address.as_slice());
    assert_eq!(&output[20..], &input);
}
