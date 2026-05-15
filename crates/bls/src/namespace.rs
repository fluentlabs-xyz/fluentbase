//! Fluent's base BLS namespace.
//!
//! Layout:
//!
//! ```text
//! [b"FLUENT_DPOS_V1_"] || [chain_id.to_be_bytes()]
//! ↑ 15 bytes              ↑ 8 bytes              = 23 bytes total
//! ```
//!
//! Per-subject suffixes (`_NOTARIZE`, `_NULLIFY`, `_FINALIZE`, `_SEED`) are
//! appended by Commonware internally — our wrapper does NOT add them.
//!
//! The literal `"FLUENT_DPOS_V1_"` is immutable for the lifetime of the V1
//! chain. Any change to the variant, scheme, curve, or canonical encoding
//! requires a hard fork with a new chain_id and namespace `"FLUENT_DPOS_V2_"`.

/// Build the base BLS namespace for a given chain.
///
/// Includes `chain_id` in the namespace to prevent cross-chain replay
/// (a signature valid on testnet must not verify on mainnet).
pub fn fluent_namespace(chain_id: u64) -> Vec<u8> {
    let mut ns = Vec::with_capacity(15 + 8);
    ns.extend_from_slice(b"FLUENT_DPOS_V1_");
    ns.extend_from_slice(&chain_id.to_be_bytes());
    ns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_namespace_layout_is_stable() {
        let ns = fluent_namespace(20994);
        assert_eq!(ns.len(), 23);
        assert_eq!(&ns[..15], b"FLUENT_DPOS_V1_");
        assert_eq!(&ns[15..], &20994u64.to_be_bytes());
    }

    #[test]
    fn fluent_namespace_distinguishes_chain_ids() {
        assert_ne!(fluent_namespace(1), fluent_namespace(2));
        assert_ne!(fluent_namespace(0), fluent_namespace(u64::MAX));
    }
}
