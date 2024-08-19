use fluentbase_sdk::{basic_entrypoint, derive::Contract, SharedAPI};

#[derive(Contract)]
pub struct WasmLoaderImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> WasmLoaderImpl<SDK> {
    pub fn deploy(&self) {}
    pub fn main(&self) {}
}

basic_entrypoint!(WasmLoaderImpl);
