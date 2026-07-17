use crate::{keccak256, Address, B256};
use alloc::vec::Vec;
use rwasm::CompilationConfig;

/// Bump when a runtime, fork, linker ABI, or lowering policy change must invalidate caches.
pub const COMPILATION_CONFIG_FINGERPRINT_VERSION: u32 = 1;
pub const IMPORT_LINKER_V1_PREVIEW_VERSION: u16 = 1;
pub const STATE_ROUTER_V1_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CompilationBackend {
    Rwasm = 1,
    Wasmtime = 2,
}

impl CompilationBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rwasm => "rwasm",
            Self::Wasmtime => "wasmtime",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CompilationConfigFingerprint {
    pub version: u32,
    pub backend: CompilationBackend,
    pub import_linker_version: u16,
    pub state_router_version: u16,
    pub config_version: u32,
    pub contract_address: Address,
    pub allow_malformed_entrypoint_func_type: bool,
    pub builtins_consume_fuel: bool,
    pub consume_fuel: bool,
    pub code_snippets: bool,
    pub consume_fuel_for_bulk_ops: bool,
    pub consume_fuel_for_params_and_locals: bool,
    pub allow_func_ref_function_types: bool,
    pub allow_start_section: bool,
    pub max_allowed_memory_pages: u32,
}

impl CompilationConfigFingerprint {
    pub fn from_config(
        config: &CompilationConfig,
        backend: CompilationBackend,
        contract_address: Address,
    ) -> Self {
        Self {
            version: COMPILATION_CONFIG_FINGERPRINT_VERSION,
            backend,
            import_linker_version: IMPORT_LINKER_V1_PREVIEW_VERSION,
            state_router_version: STATE_ROUTER_V1_VERSION,
            config_version: 1,
            contract_address,
            allow_malformed_entrypoint_func_type: config.allow_malformed_entrypoint_func_type,
            builtins_consume_fuel: config.builtins_consume_fuel,
            consume_fuel: config.consume_fuel,
            code_snippets: config.code_snippets,
            consume_fuel_for_bulk_ops: config.consume_fuel_for_bulk_ops,
            consume_fuel_for_params_and_locals: config.consume_fuel_for_params_and_locals,
            allow_func_ref_function_types: config.allow_func_ref_function_types,
            allow_start_section: config.allow_start_section,
            max_allowed_memory_pages: config.max_allowed_memory_pages,
        }
    }

    pub fn stable_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(48);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.push(self.backend as u8);
        bytes.extend_from_slice(&self.import_linker_version.to_le_bytes());
        bytes.extend_from_slice(&self.state_router_version.to_le_bytes());
        bytes.extend_from_slice(&self.config_version.to_le_bytes());
        bytes.extend_from_slice(self.contract_address.as_slice());
        bytes.push(self.allow_malformed_entrypoint_func_type as u8);
        bytes.push(self.builtins_consume_fuel as u8);
        bytes.push(self.consume_fuel as u8);
        bytes.push(self.code_snippets as u8);
        bytes.push(self.consume_fuel_for_bulk_ops as u8);
        bytes.push(self.consume_fuel_for_params_and_locals as u8);
        bytes.push(self.allow_func_ref_function_types as u8);
        bytes.push(self.allow_start_section as u8);
        bytes.extend_from_slice(&self.max_allowed_memory_pages.to_le_bytes());
        bytes
    }

    pub fn identity_hash(&self) -> B256 {
        keccak256(self.stable_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CompiledModuleCacheKey {
    pub code_hash: B256,
    pub config_fingerprint: CompilationConfigFingerprint,
}

impl CompiledModuleCacheKey {
    pub const fn new(code_hash: B256, config_fingerprint: CompilationConfigFingerprint) -> Self {
        Self {
            code_hash,
            config_fingerprint,
        }
    }
}
