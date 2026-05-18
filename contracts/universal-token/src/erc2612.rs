use alloc::vec::Vec;

use fluentbase_sdk::{
    crypto::crypto_keccak256,
    storage::{StorageMap, StorageU256},
    universal_token::*,
    Address, ContextReader, ExitCode, StorageUtils, SystemAPI, B256, B512, U256,
};
use revm_precompile::secp256k1::ecrecover;

/// Permit nonce mapping: `owner -> nonce`.
type NonceStorageMap = StorageMap<Address, StorageU256>;

/// secp256k1 curve order / 2. EIP-2 requires signatures to use low-s values.
pub(crate) const SECP256K1N_HALF: U256 = U256::from_limbs([
    0xdfe9_2f46_681b_20a0,
    0x5d57_6e73_57a4_501d,
    0xffff_ffff_ffff_ffff,
    0x7fff_ffff_ffff_ffff,
]);

#[inline(always)]
pub(crate) fn nonce_get<SDK: SystemAPI>(sdk: &mut SDK, owner: Address) -> Result<U256, ExitCode> {
    Ok(NonceStorageMap::new(NONCES_STORAGE_SLOT)
        .entry(owner)
        .get(sdk))
}

#[inline(always)]
pub(crate) fn nonce_set<SDK: SystemAPI>(
    sdk: &mut SDK,
    owner: Address,
    nonce: U256,
) -> Result<(), ExitCode> {
    NonceStorageMap::new(NONCES_STORAGE_SLOT)
        .entry(owner)
        .set_checked(sdk, nonce)
}

pub(crate) fn domain_separator_value<SDK: SystemAPI>(sdk: &mut SDK) -> Result<B256, ExitCode> {
    let token_name = sdk.storage_short_string(&NAME_STORAGE_SLOT)?;
    let name_hash = crypto_keccak256(token_name.as_bytes());

    let mut encoded = Vec::with_capacity(32 * 5);
    encoded.extend_from_slice(&EIP712_DOMAIN_TYPEHASH);
    encoded.extend_from_slice(name_hash.as_slice());
    encoded.extend_from_slice(&EIP2612_VERSION_HASH);
    encoded.extend_from_slice(
        &U256::from(sdk.context().block_chain_id()).to_be_bytes::<{ U256::BYTES }>(),
    );
    encoded.extend_from_slice(sdk.context().contract_address().into_word().as_slice());

    Ok(crypto_keccak256(&encoded))
}

pub(crate) fn permit_digest<SDK: SystemAPI>(
    sdk: &mut SDK,
    owner: Address,
    spender: Address,
    value: U256,
    nonce: U256,
    deadline: U256,
) -> Result<B256, ExitCode> {
    let mut permit_encoded = Vec::with_capacity(32 * 6);
    permit_encoded.extend_from_slice(&EIP2612_PERMIT_TYPEHASH);
    permit_encoded.extend_from_slice(owner.into_word().as_slice());
    permit_encoded.extend_from_slice(spender.into_word().as_slice());
    permit_encoded.extend_from_slice(&value.to_be_bytes::<{ U256::BYTES }>());
    permit_encoded.extend_from_slice(&nonce.to_be_bytes::<{ U256::BYTES }>());
    permit_encoded.extend_from_slice(&deadline.to_be_bytes::<{ U256::BYTES }>());
    let permit_hash = crypto_keccak256(&permit_encoded);

    let domain_separator = domain_separator_value(sdk)?;
    let mut digest_payload = Vec::with_capacity(66);
    digest_payload.extend_from_slice(b"\x19\x01");
    digest_payload.extend_from_slice(domain_separator.as_slice());
    digest_payload.extend_from_slice(permit_hash.as_slice());
    Ok(crypto_keccak256(&digest_payload))
}

pub(crate) fn ecrecover_address(digest: B256, v: u8, r: U256, s: U256) -> Option<Address> {
    if s > SECP256K1N_HALF {
        return None;
    }

    let rec_id = match v {
        27 | 28 => v - 27,
        0 | 1 => v,
        _ => return None,
    };

    let mut sig_bytes = [0u8; 64];
    sig_bytes[0..32].copy_from_slice(&r.to_be_bytes::<{ U256::BYTES }>());
    sig_bytes[32..64].copy_from_slice(&s.to_be_bytes::<{ U256::BYTES }>());
    let sig = <&B512>::try_from(&sig_bytes[..]).ok()?;

    let recovered = ecrecover(sig, rec_id, &digest).ok()?;
    let mut recovered_addr = [0u8; 20];
    recovered_addr.copy_from_slice(&recovered[12..32]);
    let recovered_addr = Address::from_slice(&recovered_addr);

    if recovered_addr == Address::ZERO {
        return None;
    }

    Some(recovered_addr)
}
