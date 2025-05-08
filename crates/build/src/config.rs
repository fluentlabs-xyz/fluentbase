use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub cargo_manifest_dir: String,
    pub stack_size: u32,
    pub output_file_name: Option<String>,
    pub features: Vec<String>,
    pub no_default_features: bool,
    pub target: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cargo_manifest_dir: env::var("CARGO_MANIFEST_DIR").unwrap(),
            stack_size: 128 * 1024,
            output_file_name: Some("lib.wasm".to_string()),
            features: vec![],
            no_default_features: true,
            target: "wasm32-unknown-unknown".to_string(),
        }
    }
}

impl Config {
    pub fn with_cargo_manifest_dir(mut self, dir: impl Into<String>) -> Self {
        self.cargo_manifest_dir = dir.into();
        self
    }

    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }
}
