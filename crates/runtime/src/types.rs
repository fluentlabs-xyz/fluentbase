use eth_trie::DB;
use fluentbase_codec::CodecError;
use fluentbase_types::{Bytes, F254};
use hashbrown::HashMap;
use rwasm::{rwasm::BinaryFormatError, Error as RwasmError};

pub trait PreimageResolver {
    fn preimage(&self, hash: &[u8; 32]) -> Option<Bytes>;
    fn preimage_size(&self, hash: &[u8; 32]) -> Option<u32>;
}

#[derive(Default)]
pub struct NonePreimageResolver;

impl PreimageResolver for NonePreimageResolver {
    fn preimage(&self, _hash: &[u8; 32]) -> Option<Bytes> {
        None
    }

    fn preimage_size(&self, _hash: &[u8; 32]) -> Option<u32> {
        None
    }
}

impl Default for Box<dyn PreimageResolver> {
    fn default() -> Self {
        Box::new(NonePreimageResolver::default())
    }
}

pub trait TrieDb {
    fn get_node(&mut self, key: &[u8]) -> Option<Bytes>;

    fn update_node(&mut self, key: &[u8], value: Bytes);

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes>;

    fn update_preimage(&mut self, key: &[u8], value: Bytes);
}

#[derive(Default, Clone)]
pub struct InMemoryTrieDb {
    nodes: HashMap<Bytes, Bytes>,
    preimages: HashMap<Bytes, Bytes>,
}

impl TrieDb for InMemoryTrieDb {
    fn get_node(&mut self, key: &[u8]) -> Option<Bytes> {
        self.nodes.get(&Bytes::copy_from_slice(key)).cloned()
    }

    fn update_node(&mut self, key: &[u8], value: Bytes) {
        self.nodes.insert(Bytes::copy_from_slice(key), value);
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes> {
        self.preimages.get(&Bytes::copy_from_slice(key)).cloned()
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes) {
        self.preimages.insert(Bytes::copy_from_slice(key), value);
    }
}

impl TrieDb for eth_trie::MemoryDB {
    fn get_node(&mut self, key: &[u8]) -> Option<Bytes> {
        self.get(key).map_or(None, |v| v.map(|v| Bytes::from(v)))
    }

    fn update_node(&mut self, key: &[u8], value: Bytes) {
        self.insert(key, value.into()).unwrap()
    }

    fn get_preimage(&mut self, key: &[u8]) -> Option<Bytes> {
        self.get(key).map_or(None, |v| v.map(|v| Bytes::from(v)))
    }

    fn update_preimage(&mut self, key: &[u8], value: Bytes) {
        self.insert(key, value.into()).unwrap()
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    BinaryFormatError(BinaryFormatError),
    Rwasm(RwasmError),
    MissingEntrypoint,
    UnloadedModule(F254),
    ExitCode(i32),
    ExecutionInterrupted,
    CodecError(CodecError),
}

impl From<BinaryFormatError> for RuntimeError {
    fn from(value: BinaryFormatError) -> Self {
        Self::BinaryFormatError(value)
    }
}

impl From<RwasmError> for RuntimeError {
    fn from(value: RwasmError) -> Self {
        Self::Rwasm(value)
    }
}

impl From<CodecError> for RuntimeError {
    fn from(value: CodecError) -> Self {
        Self::CodecError(value)
    }
}

macro_rules! rwasm_error {
    ($error_type:path) => {
        impl From<$error_type> for $crate::types::RuntimeError {
            fn from(value: $error_type) -> Self {
                Self::Rwasm(value.into())
            }
        }
    };
}

rwasm_error!(rwasm::global::GlobalError);
rwasm_error!(rwasm::memory::MemoryError);
rwasm_error!(rwasm::table::TableError);
rwasm_error!(rwasm::linker::LinkerError);
rwasm_error!(rwasm::module::ModuleError);
