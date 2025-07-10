use solana_feature_set::FeatureSet;

pub fn feature_set_default() -> FeatureSet {
    let mut feature_set = FeatureSet::all_enabled();
    // direct mapping disabled and removed from runtime
    // feature_set.deactivate(&bpf_account_data_direct_mapping::id());
    feature_set
}
