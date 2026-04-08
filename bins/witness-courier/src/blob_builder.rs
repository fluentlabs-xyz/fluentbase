//! Build EIP-4844 blobs from L2 block transactions fetched via RPC.
//!
//! Pipeline: fetch blocks → encode payload → brotli compress → canonicalize

use alloy_eips::eip2718::Encodable2718;
use alloy_provider::{Provider, RootProvider};
use brotlic::{BrotliEncoderOptions, CompressorWriter, Quality, WindowSize};
use eyre::{eyre, Result};
use std::io::Write;
use tracing::info;

/// EIP-4844 blob size: 4096 field elements × 32 bytes = 131072 bytes.
const BYTES_PER_BLOB: usize = 131_072;

const BYTES_PER_FIELD_ELEMENT: usize = 31;
const FIELD_ELEMENTS_PER_BLOB: usize = BYTES_PER_BLOB / 32;
const MAX_RAW_BYTES_PER_BLOB: usize = FIELD_ELEMENTS_PER_BLOB * BYTES_PER_FIELD_ELEMENT;

/// Fetch L2 blocks and build canonical EIP-4844 blobs.
///
/// Returns `Vec<Vec<u8>>` where each inner Vec is exactly BYTES_PER_BLOB (131072) bytes.
pub async fn build_blobs_from_l2(
    provider: &RootProvider,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Vec<u8>>> {
    let tx_data_per_block = fetch_tx_data_batched(provider, from_block, to_block).await?;
    let payload = encode_blob_payload(from_block, &tx_data_per_block);
    let compressed = brotli_compress(&payload)?;

    info!(
        from_block, to_block,
        raw_size = payload.len(),
        compressed_size = compressed.len(),
        "Blob payload compressed"
    );

    if compressed.is_empty() {
        return build_single_blob(&[0u8]).map(|b| vec![b]);
    }

    compressed
        .chunks(MAX_RAW_BYTES_PER_BLOB)
        .map(build_single_blob)
        .collect()
}

/// Fetch raw EIP-2718 tx data for a range of blocks using concurrent batching.
///
/// Returns one `Vec<u8>` per block: all transactions concatenated in block body order.
async fn fetch_tx_data_batched(
    provider: &RootProvider,
    from_block: u64,
    to_block: u64,
) -> Result<Vec<Vec<u8>>> {
    let mut result = Vec::with_capacity((to_block - from_block + 1) as usize);

    const BATCH_SIZE: u64 = 100;
    let mut current = from_block;

    while current <= to_block {
        let batch_end = (current + BATCH_SIZE - 1).min(to_block);

        let futs: Vec<_> = (current..=batch_end)
            .map(|bn| {
                let provider = provider.clone();
                async move {
                    let block = provider
                        .get_block_by_number(bn.into())
                        .full()
                        .await
                        .map_err(|e| eyre!("get_block_by_number({bn}) failed: {e}"))?
                        .ok_or_else(|| eyre!("L2 block {bn} not found"))?;

                    let mut tx_data = Vec::new();
                    for tx in block.transactions.txns() {
                        tx_data.extend_from_slice(&tx.inner.encoded_2718());
                    }
                    Ok::<(u64, Vec<u8>), eyre::Report>((bn, tx_data))
                }
            })
            .collect();

        let results = futures::future::try_join_all(futs).await?;

        for (_bn, tx_data) in results {
            result.push(tx_data);
        }

        current = batch_end + 1;
    }

    Ok(result)
}

/// Encode per-block tx data into the blob payload format (before compression).
fn encode_blob_payload(from_block: u64, tx_data_per_block: &[Vec<u8>]) -> Vec<u8> {
    let num_blocks = tx_data_per_block.len() as u32;
    let to_block = from_block + num_blocks as u64 - 1;

    let mut payload = Vec::new();
    payload.extend_from_slice(&from_block.to_be_bytes());
    payload.extend_from_slice(&to_block.to_be_bytes());
    payload.extend_from_slice(&num_blocks.to_be_bytes());
    for chunk in tx_data_per_block {
        payload.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
    }
    for chunk in tx_data_per_block {
        payload.extend_from_slice(chunk);
    }
    payload
}

/// Brotli compress via C reference libbrotli (quality=6, lgwin=22, mode=Generic).
///
/// Uses `brotlic` which compiles the C reference libbrotli from source, guaranteeing
/// byte-identical output with the Go sequencer (andybalholm/brotli is a c2go
/// translation of the same C reference). The pure-Rust brotli crate produces
/// different output on real transaction data.
fn brotli_compress(data: &[u8]) -> Result<Vec<u8>> {
    let encoder = BrotliEncoderOptions::new()
        .quality(Quality::new(6).map_err(|e| eyre!("invalid quality: {e}"))?)
        .window_size(WindowSize::new(22).map_err(|e| eyre!("invalid window size: {e}"))?)
        .build()
        .map_err(|e| eyre!("encoder build failed: {e}"))?;

    let mut writer = CompressorWriter::with_encoder(encoder, Vec::new());
    writer.write_all(data).map_err(|e| eyre!("brotli compress failed: {e}"))?;
    let compressed = writer.into_inner().map_err(|e| eyre!("brotli finalize failed: {e}"))?;
    Ok(compressed)
}

/// Canonicalize raw bytes and build a blob buffer.
fn build_single_blob(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() > MAX_RAW_BYTES_PER_BLOB {
        return Err(eyre!(
            "data ({} bytes) exceeds blob capacity ({MAX_RAW_BYTES_PER_BLOB})",
            raw.len()
        ));
    }
    Ok(canonicalize(raw))
}

/// Canonicalize raw bytes into BYTES_PER_BLOB-length buffer.
/// Each 32-byte field element: [0x00, raw[0..31]].
fn canonicalize(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; BYTES_PER_BLOB];
    let mut src_off = 0;
    let mut dst_off = 0;
    while src_off < raw.len() {
        dst_off += 1; // skip 0x00 high byte
        let take = BYTES_PER_FIELD_ELEMENT.min(raw.len() - src_off);
        out[dst_off..dst_off + take].copy_from_slice(&raw[src_off..src_off + take]);
        src_off += take;
        dst_off += BYTES_PER_FIELD_ELEMENT;
    }
    out
}
