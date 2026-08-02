//! AWS KMS custody helpers for DPoS validator secrets (host-side only):
//! - BLS consensus key: envelope-at-rest (kms:Decrypt a local CiphertextBlob → scalar).
//! - Slasher EOA: true remote signer (alloy_signer_aws::AwsSigner over kms:Sign).
//!
//! The pure-sync `fluentbase-bls` crate stays tokio/aws-free; all async KMS I/O is here.

use std::path::Path;

use eyre::{eyre, OptionExt as _, WrapErr as _};
use zeroize::Zeroizing;

/// Build a KMS client from the default AWS provider chain (env vars, IAM instance
/// profile / IMDS, IRSA / web-identity). Region comes from `AWS_REGION`/profile/IMDS
/// or an ARN key-id.
pub(crate) async fn kms_client() -> aws_sdk_kms::Client {
    let cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    aws_sdk_kms::Client::new(&cfg)
}

/// `kms:Decrypt` the local CiphertextBlob file into the 32-byte BLS scalar. Passing
/// `key_id` to Decrypt is a confused-deputy / swapped-blob defense.
pub(crate) async fn decrypt_bls_scalar(
    key_id: &str,
    ciphertext_path: &Path,
) -> eyre::Result<Zeroizing<[u8; 32]>> {
    let blob = std::fs::read(ciphertext_path)
        .wrap_err_with(|| format!("reading KMS ciphertext blob {}", ciphertext_path.display()))?;
    let out = kms_client()
        .await
        .decrypt()
        .key_id(key_id)
        .ciphertext_blob(aws_sdk_kms::primitives::Blob::new(blob))
        .send()
        .await
        .map_err(|e| eyre!("kms:Decrypt failed for key {key_id}: {e}"))?;
    let pt = out
        .plaintext()
        .ok_or_eyre("kms:Decrypt returned no plaintext")?;
    let arr: [u8; 32] = pt.as_ref().try_into().map_err(|_| {
        eyre!(
            "decrypted BLS secret is {} bytes, expected 32",
            pt.as_ref().len()
        )
    })?;
    Ok(Zeroizing::new(arr))
}

/// Async constructor for the KMS-backed slasher signer (does GetPublicKey→address).
pub(crate) async fn slasher_signer(
    key_id: &str,
    chain_id: u64,
) -> eyre::Result<alloy_signer_aws::AwsSigner> {
    alloy_signer_aws::AwsSigner::new(kms_client().await, key_id.to_string(), Some(chain_id))
        .await
        .map_err(|e| eyre!("building AWS KMS slasher signer for key {key_id}: {e}"))
}
