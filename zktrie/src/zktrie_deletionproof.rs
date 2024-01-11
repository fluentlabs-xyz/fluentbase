use std::prelude::v1::*;

use core::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    node_bytes, Byte32, Database, Error, Hash, HashScheme, Node, NodeValue, PreimageDatabase,
    ZkTrie, MAGIC_HASH, MAGIC_SMT_BYTES,
};

pub trait KeyValueWriter {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Error>;
    fn delete(&mut self, key: &[u8]) -> Result<(), Error>;
}

pub struct ProofTracer<'a, H: HashScheme> {
    trie: &'a ZkTrie<H>,
    raw_paths: BTreeMap<Vec<u8>, Vec<Arc<Node<H>>>>,
    empty_term_paths: BTreeMap<Vec<u8>, Vec<Arc<Node<H>>>>,
    deletion_tracer: BTreeSet<Hash>,
}

impl<'a, H: HashScheme> ProofTracer<'a, H> {
    pub fn new(trie: &'a ZkTrie<H>) -> Self {
        let mut deletion_tracer = BTreeSet::new();
        deletion_tracer.insert(Hash::default());
        Self {
            trie,
            raw_paths: Default::default(),
            empty_term_paths: Default::default(),
            deletion_tracer,
        }
    }

    // Merge merge the input tracer into current and return current tracer
    pub fn merge<'b>(&mut self, another: ProofTracer<'b, H>) -> &mut Self {
        // sanity checking
        if self.trie.hash() != another.trie.hash() {
            panic!("can not merge two proof tracer base on different trie");
        }

        self.deletion_tracer.extend(another.deletion_tracer);
        self.raw_paths.extend(another.raw_paths);
        self.empty_term_paths.extend(another.empty_term_paths);
        return self;
    }

    // GetDeletionProofs generate current deletionTracer and collect deletion proofs
    // which is possible to be used from all rawPaths, which enabling witness generator
    // to predict the final state root after executing any deletion
    // along any of the rawpath, no matter of the deletion occurs in any position of the mpt ops
    // Note the collected sibling node has no key along with it since witness generator would
    // always decode the node for its purpose
    pub fn get_deletion_proofs<D>(&mut self, db: &mut D) -> Result<Vec<Vec<u8>>, Error>
    where
        D: Database<Node = Node<H>>,
    {
        let mut ret_map = BTreeMap::new();

        // check each path: reversively, skip the final leaf node
        for (_, path) in &self.raw_paths {
            for n in path.iter().rev().skip(1) {
                let hash = n.hash();
                let n = n.branch().expect("should be branch node");
                let deleted_left = self.deletion_tracer.contains(n.left.hash());
                let deleted_right = self.deletion_tracer.contains(n.right.hash());
                if deleted_left && deleted_right {
                    self.deletion_tracer.insert(*hash);
                } else {
                    let sibling_hash = match () {
                        _ if deleted_left => Some(n.right.hash()),
                        _ if deleted_right => Some(n.left.hash()),
                        _ => None,
                    };
                    if let Some(sibling_hash) = sibling_hash {
                        let sibling = match self.trie.get_node(db, sibling_hash)? {
                            Some(sibling) => sibling,
                            None => return Err(Error::KeyNotFound),
                        };
                        if !sibling.is_empty() {
                            ret_map.insert(*sibling_hash, sibling.bytes());
                        }
                    }
                    break;
                }
            }
        }

        return Ok(ret_map.into_values().collect());
    }

    // MarkDeletion mark a key has been involved into deletion
    pub fn mark_deletion(&mut self, key: &[u8]) {
        let key = key.to_vec();
        match self.empty_term_paths.get(&key) {
            Some(path) => {
                self.raw_paths.insert(key, path.clone());
            }
            None => match self.raw_paths.get(&key) {
                Some(path) => {
                    assert!(path.len() > 0);
                    let leaf_node = &path[path.len() - 1];
                    assert!(leaf_node.is_leaf());
                    self.deletion_tracer.insert(*leaf_node.hash());
                }
                None => {}
            },
        }
    }

    // Prove act the same as zktrie.Prove, while also collect the raw path
    // for collecting deletion proofs in a post-work
    pub fn prove<D, S>(
        &self,
        db: &mut D,
        key: &[u8],
        from_level: usize,
        proof_kv: &mut S,
    ) -> Result<(), Error>
    where
        D: PreimageDatabase<Node = Node<H>>,
        S: KeyValueWriter,
    {
        let mpt_path = RefCell::new(vec![]);
        self.trie.prove_with_deletion(
            db,
            key,
            from_level,
            |db, node| {
                let mut key_preimage = None;
                match node.value() {
                    NodeValue::Leaf(leaf) => {
                        key_preimage = match db.preimage(&leaf.key.fr().unwrap()) {
                            preimage if preimage.len() > 0 => Some(Byte32::from_bytes(&preimage)),
                            _ => None,
                        };
                    }
                    NodeValue::Branch(_) => {
                        mpt_path.borrow_mut().push(node.clone());
                    }
                    NodeValue::Empty => {
                        mpt_path.borrow_mut().push(node.clone());
                        // emptyTermPath
                    }
                }
                proof_kv.put(node.hash().raw_bytes(), &node_bytes(&node, key_preimage))
            },
            Some(|node, _| {
                // only "hit" path (i.e. the leaf node corresponding the input key can be found)
                // would be add into tracer
                mpt_path.borrow_mut().push(node);
                // self.raw_paths.push(mpt_path.clone());
            }),
        )?;

        // we put this special kv pair in db so we can distinguish the type and
        // make suitable Proof
        proof_kv.put(MAGIC_HASH.as_ref(), MAGIC_SMT_BYTES.as_ref())?;
        Ok(())
    }
}
