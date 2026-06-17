//! Internal framing for live DKG ceremony traffic carried inside
//! [`crate::beacon::wire::BeaconMessage::Dkg`]'s opaque payload.
//!
//! The wire envelope is one opaque `Bytes` tag because the commonware DKG
//! message decoders need the round's committee size as a `Read` config; this
//! module is that typed inner framing. Each message carries its `ceremony_epoch`
//! so the single-ceremony actor can drop a stale / cross-epoch message with an
//! epoch-tag filter (no per-epoch channel demux is needed — ceremonies for E and
//! E+1 are temporally disjoint, the collection window spans ~all of E-1).
//!
//! Body variants map 1:1 to the commonware Joint-Feldman protocol steps:
//! - `Commitment` — a dealer's public polynomial commitment (broadcast).
//! - `Share` — a dealer's private share for one player (sent point-to-point).
//! - `Ack` — a player's acknowledgement of a dealer's commitment+share.
//! - `Reveal` — a dealer's signed log (public reveal for un-acked players).

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Error, Read, Write};
use commonware_cryptography::{
    bls12381::{
        dkg::{DealerPrivMsg, DealerPubMsg, PlayerAck, SignedDealerLog},
        primitives::variant::MinSig,
    },
    ed25519::PrivateKey as Ed25519PrivateKey,
};
use fluentbase_bls::PeerPubkey;
use std::num::NonZeroU32;

const TAG_COMMITMENT: u8 = 0;
const TAG_SHARE: u8 = 1;
const TAG_ACK: u8 = 2;
const TAG_REVEAL: u8 = 3;

/// A dealer's public commitment broadcast.
pub type DealerCommitment = DealerPubMsg<MinSig>;
/// A player's acknowledgement of a dealer.
pub type Ack = PlayerAck<PeerPubkey>;
/// A dealer's signed log (the public reveal for un-acked players).
pub type DealerReveal = SignedDealerLog<MinSig, Ed25519PrivateKey>;

/// One DKG ceremony protocol message.
#[derive(Clone, Debug)]
pub enum DkgBody {
    // Commitment + Reveal carry the (large) commitment polynomial / signed log;
    // boxed so the enum isn't sized to the largest variant (clippy
    // `large_enum_variant`).
    Commitment(Box<DealerCommitment>),
    Share(DealerPrivMsg),
    Ack(Ack),
    Reveal(Box<DealerReveal>),
}

/// A DKG message tagged with the ceremony epoch it belongs to (the epoch-tag
/// filter the actor uses to drop stale / cross-ceremony traffic).
#[derive(Clone, Debug)]
pub struct DkgMsg {
    pub ceremony_epoch: u64,
    pub body: DkgBody,
}

impl Write for DkgMsg {
    fn write(&self, buf: &mut impl BufMut) {
        self.ceremony_epoch.write(buf);
        match &self.body {
            DkgBody::Commitment(m) => {
                TAG_COMMITMENT.write(buf);
                m.write(buf);
            }
            DkgBody::Share(m) => {
                TAG_SHARE.write(buf);
                m.write(buf);
            }
            DkgBody::Ack(m) => {
                TAG_ACK.write(buf);
                m.write(buf);
            }
            DkgBody::Reveal(m) => {
                TAG_REVEAL.write(buf);
                m.write(buf);
            }
        }
    }
}

impl EncodeSize for DkgMsg {
    fn encode_size(&self) -> usize {
        self.ceremony_epoch.encode_size()
            + 1
            + match &self.body {
                DkgBody::Commitment(m) => m.encode_size(),
                DkgBody::Share(m) => m.encode_size(),
                DkgBody::Ack(m) => m.encode_size(),
                DkgBody::Reveal(m) => m.encode_size(),
            }
    }
}

impl Read for DkgMsg {
    /// The committee size `n` (≥ the commitment-polynomial length = quorum) used
    /// to bound the `Commitment`/`Reveal` decoders. The actor supplies it from
    /// the ceremony `Info`.
    type Cfg = NonZeroU32;

    fn read_cfg(buf: &mut impl Buf, committee_size: &NonZeroU32) -> Result<Self, Error> {
        let ceremony_epoch = u64::read_cfg(buf, &())?;
        let body = match u8::read_cfg(buf, &())? {
            TAG_COMMITMENT => {
                DkgBody::Commitment(Box::new(DealerPubMsg::read_cfg(buf, committee_size)?))
            }
            TAG_SHARE => DkgBody::Share(DealerPrivMsg::read_cfg(buf, &())?),
            TAG_ACK => DkgBody::Ack(PlayerAck::read_cfg(buf, &())?),
            TAG_REVEAL => {
                DkgBody::Reveal(Box::new(SignedDealerLog::read_cfg(buf, committee_size)?))
            }
            _ => return Err(Error::Invalid("dkg_msg", "unknown DKG message tag")),
        };
        Ok(DkgMsg {
            ceremony_epoch,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::Encode as _;
    use commonware_cryptography::{
        bls12381::{
            dkg::{Dealer, Info, Player},
            primitives::sharing::Mode,
        },
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1, NZU32};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// Re-encode equality: `DkgMsg` inner types are not all `PartialEq`, so a
    /// faithful round-trip is "decode then re-encode reproduces the bytes".
    fn assert_round_trips(msg: &DkgMsg, committee_size: NonZeroU32) {
        let encoded = msg.encode();
        assert_eq!(msg.encode_size(), encoded.len());
        let decoded = DkgMsg::read_cfg(&mut encoded.as_ref(), &committee_size).expect("decode");
        assert_eq!(decoded.ceremony_epoch, msg.ceremony_epoch);
        assert_eq!(decoded.encode(), encoded);
    }

    #[test]
    fn dkg_msg_variants_round_trip() {
        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> =
            (0..3).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
        let set = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let info = Info::<MinSig, PeerPubkey>::new::<N3f1>(
            b"ns",
            0,
            None,
            Mode::NonZeroCounter,
            set.clone(),
            set,
        )
        .expect("info");
        let n = NZU32!(3);
        let dealer_key = &keys[0];

        let (mut dealer, pub_msg, priv_msgs) =
            Dealer::start::<N3f1>(&mut rng, info.clone(), dealer_key.clone(), None).expect("start");
        assert_round_trips(
            &DkgMsg {
                ceremony_epoch: 5,
                body: DkgBody::Commitment(Box::new(pub_msg.clone())),
            },
            n,
        );

        // Drive one player through the dealer's message to mint a real Ack + log.
        let (player_pk, priv_msg) = priv_msgs
            .into_iter()
            .find(|(pk, _)| *pk != dealer_key.public_key())
            .expect("a non-dealer player");
        assert_round_trips(
            &DkgMsg {
                ceremony_epoch: 5,
                body: DkgBody::Share(priv_msg.clone()),
            },
            n,
        );

        let player_key = keys
            .iter()
            .find(|k| k.public_key() == player_pk)
            .expect("player key");
        let mut player = Player::new(info, player_key.clone()).expect("player");
        let ack = player
            .dealer_message::<N3f1>(dealer_key.public_key(), pub_msg.clone(), priv_msg)
            .expect("ack");
        assert_round_trips(
            &DkgMsg {
                ceremony_epoch: 5,
                body: DkgBody::Ack(ack.clone()),
            },
            n,
        );

        dealer
            .receive_player_ack(player_pk, ack)
            .expect("receive ack");
        let log = dealer.finalize::<N3f1>();
        assert_round_trips(
            &DkgMsg {
                ceremony_epoch: 5,
                body: DkgBody::Reveal(Box::new(log)),
            },
            n,
        );
    }

    #[test]
    fn unknown_tag_rejected() {
        let mut buf = Vec::new();
        7u64.write(&mut buf); // ceremony_epoch
        99u8.write(&mut buf); // unknown tag
        assert!(matches!(
            DkgMsg::read_cfg(&mut buf.as_slice(), &NZU32!(3)),
            Err(Error::Invalid(_, _))
        ));
    }
}
