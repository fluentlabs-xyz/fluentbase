use eth_trie::DB;
use fluentbase_codec::CodecError;
use fluentbase_rwasm::RwasmError;
use fluentbase_types::{Bytes, B256, F254};
use hashbrown::HashMap;

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

#[derive(Default)]
pub struct FixedPreimageResolver {
    preimage: Bytes,
    hash: B256,
}

impl FixedPreimageResolver {
    pub fn new(preimage: Bytes, hash: B256) -> Self {
        Self { preimage, hash }
    }
}

impl PreimageResolver for FixedPreimageResolver {
    fn preimage(&self, hash: &[u8; 32]) -> Option<Bytes> {
        assert_eq!(&self.hash.0, hash, "runtime: mismatched hash");
        Some(self.preimage.clone())
    }

    fn preimage_size(&self, hash: &[u8; 32]) -> Option<u32> {
        assert_eq!(&self.hash.0, hash, "runtime: mismatched hash");
        Some(self.preimage.len() as u32)
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
    CodecError(CodecError),
    Rwasm(RwasmError),
    UnloadedModule(F254),
    Interrupted,
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
