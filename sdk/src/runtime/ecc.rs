use k256::{
    ecdsa::{RecoveryId, Signature, VerifyingKey},
    EncodedPoint,
};

pub fn ecc_secp256k1_verify(digest: &[u8], sig: &[u8], pk_expected: &[u8], rec_id: u8) -> bool {
    let sig = Signature::from_slice(sig).unwrap();
    let pk = VerifyingKey::recover_from_prehash(
        digest,
        &sig,
        RecoveryId::new(rec_id & 0b1 > 0, rec_id & 0b10 > 0),
    )
    .unwrap();
    let pk_computed = EncodedPoint::from(&pk);
    pk_expected == pk_computed.as_bytes()
}

pub fn ecc_secp256k1_recover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: i32) -> bool {
    let sig = Signature::from_slice(sig).unwrap();
    let rec_id = RecoveryId::new(rec_id & 0b1 > 0, rec_id & 0b10 > 0);
    let pk = VerifyingKey::recover_from_prehash(digest, &sig, rec_id).unwrap();
    let pk_computed = EncodedPoint::from(&pk);
    output.copy_from_slice(pk_computed.as_bytes());
    true
}

#[cfg(test)]
mod test {
    use super::*;

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
            let res = ecc_secp256k1_verify(&digest, &vector.sig, &vector.pk, vector.recid2 as u8);
            assert_eq!(res, true);
        }
    }
}
