use eyre::WrapErr;
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    pop::sign_pop,
    PUBKEY_EIP2537_BYTES, SIGNATURE_EIP2537_BYTES,
};

pub struct PopArtefacts {
    pub bls_pubkey_uncompressed: [u8; PUBKEY_EIP2537_BYTES],
    pub bls_pop_uncompressed: [u8; SIGNATURE_EIP2537_BYTES],
}

pub fn produce(
    keypair: &ValidatorBlsKeypair,
    chain_id: u64,
) -> eyre::Result<PopArtefacts> {
    let namespace = fluent_namespace(chain_id);
    let pop_compressed = sign_pop(keypair, &namespace);
    let pub_compressed = keypair.public_bytes();

    let bls_pubkey_uncompressed = pubkey_compressed_to_eip2537(&pub_compressed)
        .map_err(|e| eyre::eyre!("BLS pubkey → EIP-2537 encoding: {e:?}"))
        .wrap_err("produce PoP")?;
    let bls_pop_uncompressed = signature_compressed_to_eip2537(&pop_compressed)
        .map_err(|e| eyre::eyre!("BLS PoP → EIP-2537 encoding: {e:?}"))
        .wrap_err("produce PoP")?;

    Ok(PopArtefacts {
        bls_pubkey_uncompressed,
        bls_pop_uncompressed,
    })
}
