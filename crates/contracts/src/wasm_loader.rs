use fluentbase_sdk::{basic_entrypoint, derive::Contract, SovereignAPI};

#[derive(Contract)]
pub struct WasmLoaderImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> WasmLoaderImpl<SDK> {
    pub fn deploy(&self) {}
    pub fn main(&self) {}
}

basic_entrypoint!(WasmLoaderImpl);
