//! The hash circuit base on poseidon.

use crate::poseidon_impl::poseidon::primitives::{
    ConstantLengthIden3,
    Domain,
    Hash,
    Spec,
    VariableLengthIden3,
};
pub use chip_short::*;
use halo2curves::bn256::Fr;
use std::fmt::Debug as DebugT;

mod chip_short {
    use crate::poseidon_impl::poseidon::{primitives::P128Pow5T3Compact, CachedConstants};

    /// The specified base hashable trait
    pub trait Hashablebase: CachedConstants {}
    /// Set the spec type as P128Pow5T3Compact
    pub type HashSpec<F> = P128Pow5T3Compact<F>;
}

/// indicate an field can be hashed in merkle tree (2 Fields to 1 Field)
pub trait Hashable: Hashablebase {
    /// the spec type used in circuit for this hashable field
    type SpecType: Spec<Self, 3, 2>;
    /// the domain type used for hash calculation
    type DomainType: Domain<Self, 2>;

    /// execute hash for any sequence of fields, with domain being specified
    fn hash_with_domain(inp: [Self; 2], domain: Self) -> Self;

    /// init a hasher used for hash
    fn hasher() -> Hash<Self, Self::SpecType, Self::DomainType, 3, 2> {
        Hash::<Self, Self::SpecType, Self::DomainType, 3, 2>::init()
    }
}

/// the domain factor applied to var-len mode hash
#[cfg(not(feature = "legacy"))]
pub const HASHABLE_DOMAIN_SPEC: u128 = 0x10000000000000000;

/// indicate an message stream constructed by the field can be hashed, commonly
/// it just need to update the Domain
pub trait MessageHashable: Hashable {
    /// the domain type used for message hash
    type DomainType: Domain<Self, 2>;
    /// hash message, if cap is not provided, it use the basic spec: (len of msg * 2^64, or len of
    /// msg in legacy mode)
    fn hash_msg(msg: &[Self], cap: Option<u128>) -> Self;
    /// init a hasher used for hash message
    fn msg_hasher(
    ) -> Hash<Self, <Self as Hashable>::SpecType, <Self as MessageHashable>::DomainType, 3, 2> {
        Hash::<Self, <Self as Hashable>::SpecType, <Self as MessageHashable>::DomainType, 3, 2>::init()
    }
}

impl Hashablebase for Fr {}

impl Hashable for Fr {
    type SpecType = HashSpec<Self>;
    type DomainType = ConstantLengthIden3<2>;

    fn hash_with_domain(inp: [Self; 2], domain: Self) -> Self {
        Self::hasher().hash(inp, domain)
    }
}

impl MessageHashable for Fr {
    type DomainType = VariableLengthIden3;

    fn hash_msg(msg: &[Self], cap: Option<u128>) -> Self {
        Self::msg_hasher()
            .hash_with_cap(msg, cap.unwrap_or(msg.len() as u128 * HASHABLE_DOMAIN_SPEC))
    }
}
