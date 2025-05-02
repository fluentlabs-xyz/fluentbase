use solana_feature_set::{bpf_account_data_direct_mapping, FeatureSet};

pub fn feature_set_default() -> FeatureSet {
    let mut feature_set = FeatureSet::all_enabled();
    feature_set.deactivate(&bpf_account_data_direct_mapping::id());
    feature_set
}
