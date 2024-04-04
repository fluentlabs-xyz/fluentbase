use eth_trie::DB;
use hashbrown::HashMap;
use rwasm::{rwasm::BinaryFormatError, Error as RwasmError};

use fluentbase_types::Bytes;

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
    StorageError(String),
    MissingEntrypoint,
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

pub type PtrAndSize = (*const u8, u32);
#[derive(Clone)]
pub enum BytecodeRepr {
    Vector(Vec<u8>),
    Unsafe(PtrAndSize),
}

impl Default for BytecodeRepr {
    fn default() -> Self {
        Self::Vector(Default::default())
    }
}

impl<const N: usize> From<&[u8; N]> for BytecodeRepr {
    fn from(value: &[u8; N]) -> Self {
        BytecodeRepr::Vector(value.into())
    }
}

impl Into<BytecodeRepr> for Vec<u8> {
    fn into(self) -> BytecodeRepr {
        BytecodeRepr::Vector(self)
    }
}

impl Into<BytecodeRepr> for PtrAndSize {
    fn into(self) -> BytecodeRepr {
        BytecodeRepr::Unsafe(self)
    }
}

impl<'t> AsRef<[u8]> for BytecodeRepr {
    fn as_ref(&self) -> &[u8] {
        match self {
            BytecodeRepr::Vector(v) => v,
            BytecodeRepr::Unsafe(v) => {
                let r = unsafe { &*core::ptr::slice_from_raw_parts(v.0, v.1 as usize) };
                r
            }
        }
    }
}
