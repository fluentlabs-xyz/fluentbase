//! `BEACON_CHANNEL` message envelope: DKG ceremony traffic.
//!
//! Per-height seed PARTIALS no longer ride this channel — they are part of the
//! consensus vote (the combined `fluentbase_bls::Scheme`), recovered from the
//! notarization/finalization certificate. This channel now carries only DKG
//! ceremony traffic (the live per-epoch DKG actor is phased).

use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{EncodeSize, Read, Write};

/// Decode cap for an opaque DKG protocol message (a signed dealer log for a
/// committee ≤ MAX_COMMITTEE_SIZE is the largest; 64 KiB is ample headroom).
pub const MAX_DKG_MSG_SIZE: usize = 64 * 1024;

const TAG_DKG: u8 = 0;

/// A message on the beacon plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeaconMessage {
    /// An opaque encoded commonware DKG protocol message (dealer public/private
    /// message, player ack, or signed dealer log). Opaque at this layer because
    /// their decode needs the round `Info`; the DKG actor parses with that context.
    Dkg(Bytes),
}

impl Write for BeaconMessage {
    fn write(&self, buf: &mut impl BufMut) {
        match self {
            BeaconMessage::Dkg(bytes) => {
                TAG_DKG.write(buf);
                (bytes.len() as u32).write(buf);
                buf.put_slice(bytes);
            }
        }
    }
}

impl EncodeSize for BeaconMessage {
    fn encode_size(&self) -> usize {
        1 + match self {
            BeaconMessage::Dkg(bytes) => 4 + bytes.len(),
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
    use commonware_codec::{Encode as _, ReadExt as _};

    #[test]
    fn dkg_envelope_round_trips() {
        let msg = BeaconMessage::Dkg(Bytes::from(vec![0xCDu8; 200]));
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
