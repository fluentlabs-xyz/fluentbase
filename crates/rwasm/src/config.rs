#[derive(Default)]
pub struct ExecutorConfig {
    pub fuel_enabled: bool,
    pub fuel_limit: Option<u64>,
    pub floats_enabled: bool,
    pub trace_enabled: bool,
}

impl ExecutorConfig {
    pub fn new() -> Self {
        Self {
            fuel_enabled: true,
            fuel_limit: None,
            floats_enabled: false,
            trace_enabled: false,
        }
    }

    pub fn fuel_enabled(mut self, fuel_enabled: bool) -> Self {
        self.fuel_enabled = fuel_enabled;
        self
    }

    pub fn fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = Some(fuel_limit);
        self
    }

    pub fn floats_enabled(mut self, floats_enabled: bool) -> Self {
        self.floats_enabled = floats_enabled;
        self
    }

    pub fn trace_enabled(mut self, trace_enabled: bool) -> Self {
        self.trace_enabled = trace_enabled;
        self
    }
}
