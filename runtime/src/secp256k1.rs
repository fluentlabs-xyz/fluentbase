extern crate alloc;

use alloc::{vec, vec::Vec};
use fluentbase_poseidon::Hashable;
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use k256::{
    ecdsa::{RecoveryId, Signature, VerifyingKey},
    EncodedPoint,
};
use sha2::{Digest, Sha256};

pub fn secp256k1_verify(digest: &[u8], sig: &[u8], recid: u8, pk_expected: &[u8]) -> bool {
    let sig = Signature::try_from(sig).unwrap();
    let recid0 = recid & 0b1 > 0;
    let recid1 = recid & 0b10 > 0;
    let recid = RecoveryId::new(recid0, recid1);
    let pk = VerifyingKey::recover_from_prehash(digest, &sig, recid).unwrap();
    let pk_computed = EncodedPoint::from(&pk);
    return pk_expected == pk_computed.as_bytes();
}

#[cfg(test)]
mod secp256k1_tests {
    extern crate alloc;

    use crate::secp256k1::secp256k1_verify;
    use hex_literal::hex;
    use k256::ecdsa::RecoveryId;
    use sha2::{Digest, Sha256};

    struct RecoveryTestVector {
        pk: [u8; 33],
        msg: &'static [u8],
        sig: [u8; 64],
        recid: RecoveryId,
        recid2: usize,
    }

    const RECOVERY_TEST_VECTORS: &[RecoveryTestVector] = &[
        // Recovery ID 0
        RecoveryTestVector {
            pk: hex!("021a7a569e91dbf60581509c7fc946d1003b60c7dee85299538db6353538d59574"),
            msg: b"example message",
            sig: hex!(
                "ce53abb3721bafc561408ce8ff99c909f7f0b18a2f788649d6470162ab1aa0323971edc523a6d6453f3fb6128d318d9db1a5ff3386feb1047d9816e780039d52"
            ),
            recid: RecoveryId::new(false, false),
            recid2: 0,
        },
        // Recovery ID 1
        RecoveryTestVector {
            pk: hex!("036d6caac248af96f6afa7f904f550253a0f3ef3f5aa2fe6838a95b216691468e2"),
            msg: b"example message",
            sig: hex!(
                "46c05b6368a44b8810d79859441d819b8e7cdc8bfd371e35c53196f4bcacdb5135c7facce2a97b95eacba8a586d87b7958aaf8368ab29cee481f76e871dbd9cb"
            ),
            recid: RecoveryId::new(true, false),
            recid2: 1,
        },
    ];

    #[test]
    fn public_key_recovery() {
        for vector in RECOVERY_TEST_VECTORS {
            let digest = Sha256::new_with_prefix(vector.msg).finalize();

            let mut params_vec: Vec<u8> = vec![];
            params_vec.extend(&digest);
            params_vec.extend(&vector.sig);
            params_vec.push(vector.recid2 as u8);
            params_vec.extend(&vector.pk);
            println!("params_vec {:?} len {}", params_vec, params_vec.len());

            let res = secp256k1_verify(&digest, &vector.sig, vector.recid2 as u8, &vector.pk);
            assert_eq!(res, true);
        }
    }
}
