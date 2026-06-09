//! Cache layer: commonware-codec for [`ValidatorSetSnapshot`] + the
//! [`ValidatorSetCache`] — a write-once `commonware_storage::prunable::Archive`
//! (finalized-only, `Index = epoch`, `Key = block_hash`). Generic over a
//! commonware runtime context `E`; the node builds a
//! `commonware_runtime::tokio` context rooted at
//! `<reth_datadir>/staking-reader/` and injects it.
//!
//! The speculative in-mem hot tier was removed: under finality-gated apply
//! there is no tentative window and no sync `get_hot` consumer; re-add
//! shaped by a real consumer if one ever appears.
//!
//! Decode-on-load through the subgroup-checked `fluentbase-bls` decoders
//! *is* the integrity check.

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Error as CodecError, Read, ReadExt as _, Write};

use alloy_primitives::{Address, B256};
use commonware_runtime::{buffer::paged::CacheRef, BufferPooler, Metrics, Storage};
use commonware_storage::{
    archive::{prunable, Archive as _, Identifier},
    translator::FourCap,
};
use commonware_utils::{sequence::FixedBytes, NZUsize, NZU16, NZU64};
use fluentbase_bls::{BlsPubkey, PeerPubkey, PUBKEY_BYTES};

use crate::{
    error::ReadError,
    reader::{ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys},
};

// snapshot codec

/// Sanity bound when decoding the (locally-stored, non-authoritative)
/// validators vector. The on-chain committee is a bounded top-k; this is
/// only a guard against a corrupt length prefix.
pub(crate) const MAX_VALIDATORS: usize = 4096;

const HASH_BYTES: usize = 32;
const ADDR_BYTES: usize = 20;
const PEER_BYTES: usize = 32;

impl Write for ValidatorSetSnapshot {
    fn write(&self, buf: &mut impl BufMut) {
        buf.put_slice(self.block_hash.as_slice());
        buf.put_u64(self.block_number);
        buf.put_u64(self.epoch);
        buf.put_u32(self.validators.len() as u32);
        for v in &self.validators {
            buf.put_slice(v.address.as_slice());
            v.keys.bls_pubkey.write(buf);
            v.keys.peer_pubkey.write(buf);
            buf.put_u64(v.keys.activation_epoch);
        }
    }
}

impl EncodeSize for ValidatorSetSnapshot {
    fn encode_size(&self) -> usize {
        HASH_BYTES
            + 8
            + 8
            + 4
            + self.validators.len() * (ADDR_BYTES + PUBKEY_BYTES + PEER_BYTES + 8)
    }
}

impl Read for ValidatorSetSnapshot {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _cfg: &()) -> Result<Self, CodecError> {
        if buf.remaining() < HASH_BYTES + 8 + 8 + 4 {
            return Err(CodecError::EndOfBuffer);
        }
        let mut h = [0u8; HASH_BYTES];
        buf.copy_to_slice(&mut h);
        let block_hash = B256::from(h);
        let block_number = buf.get_u64();
        let epoch = buf.get_u64();
        let len = buf.get_u32() as usize;
        if len > MAX_VALIDATORS {
            return Err(CodecError::InvalidLength(len));
        }

        let mut validators = Vec::with_capacity(len);
        for _ in 0..len {
            if buf.remaining() < ADDR_BYTES {
                return Err(CodecError::EndOfBuffer);
            }
            let mut a = [0u8; ADDR_BYTES];
            buf.copy_to_slice(&mut a);
            let address = Address::from(a);
            // Subgroup-checked decode (integrity check).
            let bls_pubkey = BlsPubkey::read(buf)?;
            let peer_pubkey = PeerPubkey::read(buf)?;
            if buf.remaining() < 8 {
                return Err(CodecError::EndOfBuffer);
            }
            let activation_epoch = buf.get_u64();
            validators.push(ValidatorWithKeys {
                address,
                keys: ConsensusKeys {
                    bls_pubkey,
                    peer_pubkey,
                    activation_epoch,
                },
            });
        }
        Ok(ValidatorSetSnapshot {
            block_hash,
            block_number,
            epoch,
            validators,
        })
    }
}

type Key = FixedBytes<32>;
type Inner<E> = prunable::Archive<FourCap, E, Key, ValidatorSetSnapshot>;

#[inline]
fn key_of(block_hash: B256) -> Key {
    FixedBytes::new(block_hash.0)
}

/// Durable validator-set store: a write-once epoch-indexed
/// `prunable::Archive` (no hot tier). Strictly
/// `block_hash`-keyed; it does NOT track which hash is canonical for an
/// epoch (that pointer is `epoch_transition`'s).
pub struct ValidatorSetCache<E: Storage + Metrics + BufferPooler> {
    archive: Inner<E>,
}

impl<E: Storage + Metrics + BufferPooler> ValidatorSetCache<E> {
    /// `context` is rooted + metrics-labelled by the node
    /// (e.g. `node_ctx.with_label("staking-reader-cache")`) at
    /// `<reth_datadir>/staking-reader/`; this fn does NOT add its own metrics
    /// label (doing so would double-register on a same-namespace re-init).
    /// Retention is driven by `prune(min_epoch)` calls — no hot tier.
    pub async fn init(context: E) -> Result<Self, ReadError> {
        let cfg = prunable::Config {
            translator: FourCap,
            key_partition: "vsc-key".into(),
            key_page_cache: CacheRef::from_pooler(&context, NZU16!(4096), NZUsize!(256)),
            value_partition: "vsc-val".into(),
            codec_config: (),
            compression: None,
            key_write_buffer: NZUsize!(1 << 20),
            value_write_buffer: NZUsize!(1 << 20),
            replay_buffer: NZUsize!(1 << 20),
            // One epoch per section ⇒ `prune(min_epoch)` is exact (validator
            // sets are written at most once per epoch).
            items_per_section: NZU64!(1),
        };
        let archive = prunable::Archive::init(context, cfg)
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))?;
        Ok(Self { archive })
    }

    /// Persist a FINALIZED snapshot to the durable archive.
    ///
    /// Idempotence: `prunable::Archive::put` is **already idempotent**
    /// on duplicate index — see
    /// `commonware_storage::archive::prunable::storage.rs:332-334`,
    /// `put_internal(..., skip_if_index_exists=true)`. A re-call with the
    /// same `epoch` (typical retry path where `try_send` returned `Full`
    /// and `last_tracked_epoch` was deliberately not advanced) silently
    /// returns Ok without rewriting, so crash-recovery cannot brick the
    /// node on a duplicate persist. Unlike `immutable::Archive` (which
    /// DOES error on duplicate), `prunable::Archive` is idempotent here
    /// from the start.
    ///
    /// `Index = snapshot.epoch`, `Key = block_hash`. Caller
    /// (`epoch_transition`) must only call this once the epoch's
    /// committing block is final.
    pub async fn persist_final(&mut self, snapshot: ValidatorSetSnapshot) -> Result<(), ReadError> {
        self.archive
            .put(snapshot.epoch, key_of(snapshot.block_hash), snapshot)
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))?;
        self.archive
            .sync()
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))?;
        Ok(())
    }

    /// Durable archive lookup by `block_hash`.
    pub async fn get(&self, block_hash: B256) -> Result<Option<ValidatorSetSnapshot>, ReadError> {
        self.archive
            .get(Identifier::Key(&key_of(block_hash)))
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))
    }

    /// Durable archive lookup by `epoch`.
    ///
    /// Stale-epoch fallback path: the slasher uses this when
    /// `Staking.getEpochCommittee(epoch)` on-chain returns empty (the
    /// contract's prune cursor has advanced past the evidence epoch).
    /// `prunable::Archive::get(Identifier::Index(epoch))` is directly
    /// supported — no side index needed (the archive's own `Index = epoch`
    /// mapping is the canonical lookup).
    pub async fn get_by_epoch(
        &self,
        epoch: u64,
    ) -> Result<Option<ValidatorSetSnapshot>, ReadError> {
        self.archive
            .get(Identifier::Index(epoch))
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))
    }

    /// Whether `block_hash` is in the durable archive.
    pub async fn contains(&self, block_hash: B256) -> Result<bool, ReadError> {
        self.archive
            .has(Identifier::Key(&key_of(block_hash)))
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))
    }

    /// Drop archived epochs `< min_epoch`.
    ///
    /// Cursor parity: mirrors the on-chain `_pruneStaleCommittees`
    /// cursor advance (`solidity-contracts/contracts/staking/Staking.sol`
    /// `_pruneStaleCommittees`). Because `archive.prune(min_epoch)` drops
    /// everything below `min_epoch` regardless of whether each individual
    /// epoch had a committed snapshot, the cache cursor advances through
    /// skipped commits in lock-step with the on-chain `prunedUpToP1` — no
    /// per-epoch cursor tracking on the Rust side.
    pub async fn prune(&mut self, min_epoch: u64) -> Result<(), ReadError> {
        self.archive
            .prune(min_epoch)
            .await
            .map_err(|e| ReadError::Backend(e.to_string()))
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;
    use commonware_codec::Encode as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    /// Arbitrary fixed seed for deterministic test fixtures (no semantics).
    const TEST_SEED: u64 = 1;

    fn snapshot(n: usize) -> ValidatorSetSnapshot {
        let mut rng = StdRng::seed_from_u64(TEST_SEED);
        let validators = (0..n)
            .map(|i| {
                let peer = Ed25519PrivateKey::random(&mut rng).public_key();
                let bls = {
                    use commonware_codec::DecodeExt as _;
                    BlsPubkey::decode(
                        fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng)
                            .public_bytes()
                            .as_slice(),
                    )
                    .unwrap()
                };
                ValidatorWithKeys {
                    address: Address::repeat_byte(i as u8),
                    keys: ConsensusKeys {
                        bls_pubkey: bls,
                        peer_pubkey: peer,
                        activation_epoch: 3 + i as u64,
                    },
                }
            })
            .collect();
        ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0xBB),
            block_number: 1024,
            epoch: 7,
            validators,
        }
    }

    #[test]
    fn round_trip() {
        let s = snapshot(3);
        let bytes = s.encode();
        let back = ValidatorSetSnapshot::read(&mut bytes.as_ref()).expect("decode");
        assert_eq!(back.block_hash, s.block_hash);
        assert_eq!(back.block_number, s.block_number);
        assert_eq!(back.epoch, s.epoch);
        assert_eq!(back.validators.len(), 3);
        assert_eq!(back.validators[1].address, s.validators[1].address);
        assert_eq!(
            back.validators[2].keys.activation_epoch,
            s.validators[2].keys.activation_epoch
        );
    }

    #[test]
    fn empty_validators_round_trip() {
        let s = snapshot(0);
        let bytes = s.encode();
        let back = ValidatorSetSnapshot::read(&mut bytes.as_ref()).expect("decode");
        assert!(back.validators.is_empty());
    }

    #[test]
    fn truncated_buffer_errors() {
        let s = snapshot(2);
        let bytes = s.encode();
        let truncated = &bytes[..bytes.len() - 10];
        assert!(ValidatorSetSnapshot::read(&mut &truncated[..]).is_err());
    }

    #[test]
    fn tampered_bls_key_rejected_by_subgroup_check() {
        let s = snapshot(1);
        let mut bytes = s.encode().to_vec();
        // [32 hash][8 num][8 epoch][4 len][20 addr] then 96B bls.
        let bls_off = 32 + 8 + 8 + 4 + 20;
        for b in &mut bytes[bls_off..bls_off + PUBKEY_BYTES] {
            *b = 0xFF;
        }
        assert!(ValidatorSetSnapshot::read(&mut &bytes[..]).is_err());
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use commonware_codec::DecodeExt as _;
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    fn snap(epoch: u64, hash: u8) -> ValidatorSetSnapshot {
        let mut rng = StdRng::seed_from_u64(epoch);
        let peer = Ed25519PrivateKey::random(&mut rng).public_key();
        let bls = BlsPubkey::decode(
            fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut rng)
                .public_bytes()
                .as_slice(),
        )
        .unwrap();
        ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(hash),
            block_number: epoch * 100,
            epoch,
            validators: vec![ValidatorWithKeys {
                address: Address::repeat_byte(hash),
                keys: ConsensusKeys {
                    bls_pubkey: bls,
                    peer_pubkey: peer,
                    activation_epoch: epoch + 1,
                },
            }],
        }
    }

    #[test]
    fn persist_final_then_get_and_prune() {
        deterministic::Runner::default().start(|ctx| async move {
            let mut c = ValidatorSetCache::init(ctx).await.unwrap();
            let old = snap(2, 0x22);
            let new = snap(9, 0x99);
            c.persist_final(old.clone()).await.unwrap();
            c.persist_final(new.clone()).await.unwrap();
            assert!(c.contains(old.block_hash).await.unwrap());
            c.prune(5).await.unwrap();
            assert!(c.get(new.block_hash).await.unwrap().is_some());
        });
    }

    #[test]
    fn persist_final_is_idempotent_on_duplicate_epoch() {
        // Re-calling persist_final with the same (epoch, hash) must not
        // error — `prunable::Archive::put` is documented to skip duplicate
        // indices, so the second call is a silent no-op. This protects the
        // retry path where on_finalized re-enters after `try_send` Full.
        deterministic::Runner::default().start(|ctx| async move {
            let mut c = ValidatorSetCache::init(ctx).await.unwrap();
            let s = snap(7, 0x77);
            c.persist_final(s.clone()).await.expect("first put");
            c.persist_final(s.clone())
                .await
                .expect("second put must be silent no-op");
            // Lookup still works via either key OR index.
            assert!(c.contains(s.block_hash).await.unwrap());
            assert!(c.get_by_epoch(7).await.unwrap().is_some());
        });
    }

    #[test]
    fn get_by_epoch_returns_persisted_snapshot() {
        // Slasher's stale-epoch fallback path looks up by
        // epoch (not block hash). Verify the wrapper around the archive's
        // native Identifier::Index lookup returns the expected snapshot.
        deterministic::Runner::default().start(|ctx| async move {
            let mut c = ValidatorSetCache::init(ctx).await.unwrap();
            let s = snap(11, 0x11);
            c.persist_final(s.clone()).await.unwrap();
            let got = c
                .get_by_epoch(11)
                .await
                .unwrap()
                .expect("epoch 11 must be in cache");
            assert_eq!(got.epoch, 11);
            assert_eq!(got.block_hash, s.block_hash);
            assert!(
                c.get_by_epoch(99).await.unwrap().is_none(),
                "miss returns None"
            );
        });
    }

    #[test]
    fn restart_round_trip_from_archive() {
        deterministic::Runner::default().start(|ctx| async move {
            let s = snap(4, 0x44);
            {
                // distinct metrics namespace, SAME storage partitions
                let mut c = ValidatorSetCache::init(ctx.with_label("run1"))
                    .await
                    .unwrap();
                c.persist_final(s.clone()).await.unwrap();
            } // drop cache (archive) — simulate restart
            let c2 = ValidatorSetCache::init(ctx.with_label("run2"))
                .await
                .unwrap();
            let got = c2.get(s.block_hash).await.unwrap().expect("from archive");
            assert_eq!(got.epoch, 4);
            assert_eq!(got.validators.len(), 1);
        });
    }
}
