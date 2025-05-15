use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub cargo_manifest_dir: String,
    pub stack_size: u32,
    pub output_file_name: Option<String>,
    pub features: Vec<String>,
    pub no_default_features: bool,
    pub target: String,
    pub rerun_if_changed: Vec<String>,
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
            rerun_if_changed: vec![],
        }
    }
}

impl Config {
    pub fn with_rerun_if_changed(mut self, path: &str) -> Self {
        self.rerun_if_changed.push(path.to_string());
        self
    }

    pub fn with_output_file_name(mut self, filename: Option<String>) -> Self {
        self.output_file_name = filename;
        self
    }

    pub fn with_stack_size(mut self, stack_size: u32) -> Self {
        self.stack_size = stack_size;
        self
    }

    pub fn with_features(mut self, features: Vec<String>) -> Self {
        self.features = features;
        self
    }

    pub fn with_no_default_features(mut self, no_default_features: bool) -> Self {
        self.no_default_features = no_default_features;
        self
    }
}
