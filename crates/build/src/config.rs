use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub parent_manifest_dir: String,
    pub contract_dir_name: String,
    pub stack_size: u32,
    pub output_file_name: Option<String>,
    pub features: Vec<String>,
    pub no_default_features: bool,
    pub target: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            parent_manifest_dir: env::var("CARGO_MANIFEST_DIR").unwrap(),
            contract_dir_name: "contract".to_string(),
            stack_size: 128 * 1024,
            output_file_name: Some("lib.wasm".to_string()),
            features: vec![],
            no_default_features: true,
            target: "wasm32-unknown-unknown".to_string(),
        }
    }
}
