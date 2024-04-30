use alloy_rlp::{RlpEncodable, RlpMaxEncodedLen};
use hash_db::Hasher;
use plain_hasher::PlainHasher;
use revm::{
    db::PlainAccount,
    primitives::{keccak256, Address, Log, B256, U256},
};
use triehash::sec_trie_root;

pub fn log_rlp_hash(logs: &[Log]) -> B256 {
    let mut out = Vec::with_capacity(alloy_rlp::list_length(logs));
    alloy_rlp::encode_list(logs, &mut out);
    keccak256(&out)
}

pub fn state_merkle_trie_root<'a>(
    accounts: impl IntoIterator<Item = (Address, &'a PlainAccount)>,
) -> B256 {
    trie_root(accounts.into_iter().map(|(address, acc)| {
        (
            address,
            alloy_rlp::encode_fixed_size(&TrieAccount::new(acc)),
        )
    }))
}

pub fn state_merkle_trie_root2<'a>(
    accounts: impl IntoIterator<Item = (Address, &'a fluentbase_revm::db::PlainAccount)>,
) -> B256 {
    trie_root(accounts.into_iter().map(|(address, acc)| {
        (
            address,
            alloy_rlp::encode_fixed_size(&TrieAccount::new2(acc)),
        )
    }))
}

// pub fn state_merkle_trie_root_original<'a>(
//     accounts: impl IntoIterator<Item=(ro::precompile::Address, &'a ro::db::PlainAccount)>,
// ) -> ro::precompile::B256 {
//     let res = trie_root(accounts.into_iter().map(|(address, acc)| {
//         (
//             address,
//             alloy_rlp::encode_fixed_size(&TrieAccountOriginal::new(acc)),
//         )
//     }));
//     ro::precompile::B256::from_slice(res.as_slice())
// }

#[derive(RlpEncodable, RlpMaxEncodedLen)]
struct TrieAccount {
    nonce: u64,
    balance: U256,
    root_hash: B256,
    code_hash: B256,
}

#[derive(RlpEncodable, RlpMaxEncodedLen)]
struct TrieAccountOriginal {
    nonce: u64,
    balance: U256,
    root_hash: B256,
    code_hash: revm::primitives::B256,
}

impl TrieAccount {
    fn new(acc: &PlainAccount) -> Self {
        Self {
            nonce: acc.info.nonce,
            balance: acc.info.balance,
            root_hash: sec_trie_root::<KeccakHasher, _, _, _>(
                acc.storage
                    .iter()
                    .filter(|(_k, &v)| v != U256::ZERO)
                    .map(|(k, v)| (k.to_be_bytes::<32>(), alloy_rlp::encode_fixed_size(v))),
            ),
            code_hash: acc.info.code_hash,
        }
    }
    fn new2(acc: &fluentbase_revm::db::PlainAccount) -> Self {
        Self {
            nonce: acc.info.nonce,
            balance: acc.info.balance,
            root_hash: sec_trie_root::<KeccakHasher, _, _, _>(
                acc.storage
                    .iter()
                    .filter(|(_k, &v)| v != U256::ZERO)
                    .map(|(k, v)| (k.to_be_bytes::<32>(), alloy_rlp::encode_fixed_size(v))),
            ),
            code_hash: acc.info.code_hash,
        }
    }
}

#[inline]
pub fn trie_root<I, A, B>(input: I) -> B256
where
    I: IntoIterator<Item = (A, B)>,
    A: AsRef<[u8]>,
    B: AsRef<[u8]>,
{
    sec_trie_root::<KeccakHasher, _, _, _>(input)
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeccakHasher;

impl Hasher for KeccakHasher {
    type Out = B256;
    type StdHasher = PlainHasher;
    const LENGTH: usize = 32;

    #[inline]
    fn hash(x: &[u8]) -> Self::Out {
        keccak256(x)
    }
}
