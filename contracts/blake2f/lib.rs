use fluentbase_sdk::include_this_wasm;

pub const WASM_BYTECODE: &[u8] = include_this_wasm!();

#[cfg(test)]
mod tests {
    use fluentbase_sdk_testing::HostTestingContext;
}
