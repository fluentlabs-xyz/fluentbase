use solana_feature_set::FeatureSet;

pub fn feature_set_default() -> FeatureSet {
    let feature_set = FeatureSet::all_enabled();
    feature_set
}
