//! `BEACON_CHANNEL` message envelope: DKG ceremony traffic and per-height seed
//! partials share one per-epoch-muxed channel under a `{Dkg|Seed}` tag so the
//! DKG-for-E and seed-of-E flows never interleave across epochs (critic G3).

use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{EncodeSize, Read, Write};
use commonware_cryptography::bls12381::primitives::variant::{MinSig, PartialSignature};

/// Decode cap for an opaque DKG protocol message (a signed dealer log for a
/// committee ≤ MAX_COMMITTEE_SIZE is the largest; 64 KiB is ample headroom).
pub const MAX_DKG_MSG_SIZE: usize = 64 * 1024;

const TAG_DKG: u8 = 0;
const TAG_SEED: u8 = 1;

/// A message on the beacon plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeaconMessage {
    /// An opaque encoded commonware DKG protocol message (dealer public/private
    /// message, player ack, or signed dealer log). Opaque at this layer because
    /// their decode needs the round `Info`; the DKG actor parses with that context.
    Dkg(Bytes),
    /// A per-height seed partial signature, broadcast by a committee member only
    /// AFTER it observes `height` ordering-finalized (sign-after-finalize).
    SeedPartial {
        height: u64,
        partial: PartialSignature<MinSig>,
    },
}

impl Write for BeaconMessage {
    fn write(&self, buf: &mut impl BufMut) {
        match self {
            BeaconMessage::Dkg(bytes) => {
                TAG_DKG.write(buf);
                (bytes.len() as u32).write(buf);
                buf.put_slice(bytes);
            }
            BeaconMessage::SeedPartial { height, partial } => {
                TAG_SEED.write(buf);
                height.write(buf);
                partial.write(buf);
            }
        }
    }
}

impl EncodeSize for BeaconMessage {
    fn encode_size(&self) -> usize {
        1 + match self {
            BeaconMessage::Dkg(bytes) => 4 + bytes.len(),
            BeaconMessage::SeedPartial { partial, .. } => 8 + partial.encode_size(),
        }
    }
}

impl Read for BeaconMessage {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        match u8::read_cfg(buf, &())? {
            TAG_DKG => {
                let len = u32::read_cfg(buf, &())? as usize;
                if len > MAX_DKG_MSG_SIZE {
                    return Err(commonware_codec::Error::Invalid(
                        "beacon_wire",
                        "dkg message exceeds MAX_DKG_MSG_SIZE",
                    ));
                }
                if len > buf.remaining() {
                    return Err(commonware_codec::Error::EndOfBuffer);
                }
                Ok(BeaconMessage::Dkg(buf.copy_to_bytes(len)))
            }
            TAG_SEED => {
                let height = u64::read_cfg(buf, &())?;
                let partial = PartialSignature::<MinSig>::read_cfg(buf, &())?;
                Ok(BeaconMessage::SeedPartial { height, partial })
            }
            _ => Err(commonware_codec::Error::Invalid(
                "beacon_wire",
                "unknown beacon message tag",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::seed::{seed_namespace, sign_seed_partial};
    use commonware_codec::{Encode as _, ReadExt as _};
    use commonware_cryptography::bls12381::dkg::deal_anonymous;
    use commonware_utils::{test_rng, N3f1, NZU32};

    #[test]
    fn dkg_envelope_round_trips() {
        let msg = BeaconMessage::Dkg(Bytes::from(vec![0xCDu8; 200]));
        let encoded = msg.encode();
        assert_eq!(msg.encode_size(), encoded.len());
        assert_eq!(BeaconMessage::read(&mut encoded.as_ref()).unwrap(), msg);
    }

    #[test]
    fn seed_partial_envelope_round_trips() {
        let mut rng = test_rng();
        let (_sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(4));
        let ns = seed_namespace(b"fluent-devnet");
        let partial = sign_seed_partial(&shares[0], &ns, 77);

        let msg = BeaconMessage::SeedPartial {
            height: 77,
            partial,
        };
        let encoded = msg.encode();
        assert_eq!(msg.encode_size(), encoded.len());
        assert_eq!(BeaconMessage::read(&mut encoded.as_ref()).unwrap(), msg);
    }

    #[test]
    fn oversize_dkg_message_is_rejected() {
        let mut buf = Vec::new();
        0u8.write(&mut buf); // TAG_DKG
        ((MAX_DKG_MSG_SIZE + 1) as u32).write(&mut buf);
        buf.resize(buf.len() + MAX_DKG_MSG_SIZE + 1, 0);
        assert!(matches!(
            BeaconMessage::read(&mut buf.as_slice()),
            Err(commonware_codec::Error::Invalid(_, _))
        ));
    }
}
