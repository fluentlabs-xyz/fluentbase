use crate::{
    rwasm::{
        bindings::{_zktrie_get, _zktrie_open, _zktrie_root, _zktrie_update},
        LowLevelSDK,
    },
    sdk::LowLevelZkTrieSDK,
    types::Bytes32,
};

impl LowLevelZkTrieSDK for LowLevelSDK {
    fn zktrie_open(root: &Bytes32) -> u32 {
        unsafe { _zktrie_open(root.as_ptr()) }
    }

    fn zktrie_update(trie: u32, key: &Bytes32, flags: u32, values: &[Bytes32]) {
        // unsafe {
        //     _zktrie_update(
        //         trie,
        //         key.as_ptr(),
        //         flags,
        //         values.as_ptr().as_ptr(),
        //         values.len() as u32,
        //     )
        // }
    }

    fn zktrie_get(trie: u32, key: &Bytes32, output: &mut [Bytes32]) {
        // unsafe { _zktrie_get(trie, key.as_ptr(), output.as_mut_ptr().as_mut_ptr()) }
    }

    fn zktrie_root(trie: u32, output: &mut Bytes32) {
        unsafe { _zktrie_root(trie, output.as_mut_ptr()) }
    }
}
