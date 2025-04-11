use std::env;

#[derive(Debug, Clone)]
pub struct WasmBuildConfig {
    pub cargo_manifest_dir: String,
    pub current_target: String,
    pub is_tarpaulin_build: bool,
    pub stack_size: u32,
    pub output_file_name: String,
    pub features: Vec<String>,
    pub no_default_features: bool,
    pub target: String,
    pub profile: String,
}

impl Default for WasmBuildConfig {
    fn default() -> Self {
        Self {
            cargo_manifest_dir: env::var("CARGO_MANIFEST_DIR").unwrap(),
            current_target: env::var("TARGET").unwrap(),
            is_tarpaulin_build: env::var("CARGO_CFG_TARPAULIN").is_ok(),
            stack_size: 128 * 1024,
            output_file_name: "lib.wasm".to_string(),
            features: vec![],
            no_default_features: true,
            target: "wasm32-unknown-unknown".to_string(),
            profile: env::var("PROFILE").unwrap(),
        }
    }
}

impl WasmBuildConfig {
    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    pub fn with_taget(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }
}
