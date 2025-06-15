#[derive(Debug, Clone)]
pub struct RustToWasmConfig {
    pub stack_size: u32,
    pub features: Vec<String>,
    pub no_default_features: bool,
}

impl Default for RustToWasmConfig {
    fn default() -> Self {
        Self {
            stack_size: 128 * 1024,
            features: vec![],
            no_default_features: true,
        }
    }
}

impl RustToWasmConfig {
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
