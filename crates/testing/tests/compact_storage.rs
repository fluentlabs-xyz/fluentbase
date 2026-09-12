use fluentbase_sdk::{storage_legacy::StorageValueFluent, FixedBytes, U256};
use fluentbase_testing::TestingContextImpl;

#[test]
fn legacy_storage_roundtrips_odd_width_compact_vectors() {
    let values = vec![
        FixedBytes::<11>::from([0x11; 11]),
        FixedBytes::<11>::from([0x22; 11]),
    ];
    let mut sdk = TestingContextImpl::default();
    let slot = U256::from(7);
    <Vec<FixedBytes<11>> as StorageValueFluent<_, _>>::set(&mut sdk, slot, values.clone());
    let restored = <Vec<FixedBytes<11>> as StorageValueFluent<_, _>>::get(&sdk, slot);
    assert_eq!(restored, values);
}
