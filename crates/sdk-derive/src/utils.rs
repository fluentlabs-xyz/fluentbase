use syn::{self, ImplItem, ImplItemFn, ItemImpl, parse::Parse, Visibility};

pub fn get_all_methods(ast: &ItemImpl) -> Vec<&ImplItemFn> {
    ast.items
        .iter()
        .filter_map(|item| {
            if let ImplItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect()
}

pub fn get_public_methods(ast: &ItemImpl) -> Vec<&ImplItemFn> {
    get_all_methods(ast)
        .into_iter()
        .filter(|func| matches!(func.vis, Visibility::Public(_)))
        .collect()
}

pub fn calculate_keccak256_id(signature: &str) -> u32 {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(signature);
    let mut dst = [0u8; 4];
    dst.copy_from_slice(hash.finalize().as_slice()[0..4].as_ref());
    u32::from_be_bytes(dst)
}
