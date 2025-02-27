#[derive(Default)]
pub struct ExecutorConfig {
    pub fuel_limit: Option<u64>,
    pub floats_enabled: bool,
    pub tracer_enabled: bool,
}

impl ExecutorConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = Some(fuel_limit);
        self
    }

    pub fn floats_enabled(mut self, floats_enabled: bool) -> Self {
        self.floats_enabled = floats_enabled;
        self
    }

    pub fn tracer_enabled(mut self, tracer_enabled: bool) -> Self {
        self.tracer_enabled = tracer_enabled;
        self
    }
}
