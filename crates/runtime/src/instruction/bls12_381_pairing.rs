use crate::RuntimeContext;
use blstrs::{pairing, Compress as _, G1Affine, G2Affine, Gt};
use group::Group;
use rwasm::{Store, TrapCode, TypedCaller, Value};

const G1_COMPRESSED_SIZE: usize = 48;
const G2_COMPRESSED_SIZE: usize = 96;
const GT_COMPRESSED_SIZE: usize = 288;

pub struct SyscallBls12381Pairing;

/// Pairing call expects 384*k (k being a positive integer) bytes as an inputs
/// that is interpreted as byte concatenation of k slices. Each slice has the
/// following structure:
///    * 128 bytes of G1 point encoding
///    * 256 bytes of G2 point encoding
///
/// Each point is expected to be in the subgroup of order q.
/// Output is 32 bytes where first 31 bytes are equal to 0x00 and the last byte
/// is 0x01 if pairing result is equal to the multiplicative identity in a pairing
/// target field and 0x00 otherwise.
impl SyscallBls12381Pairing {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let pairs_ptr = params[0].i32().unwrap() as usize;
        let pairs_len = params[1].i32().unwrap() as usize; // number of pairs
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair is 144 bytes: 48 bytes G1 + 96 bytes G2 (blstrs compressed)
        let total_len = pairs_len * (G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE);
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // Parse into vector of pairs
        let mut pairs: Vec<([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])> =
            Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * (G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE);
            let mut g1 = [0u8; G1_COMPRESSED_SIZE];
            let mut g2 = [0u8; G2_COMPRESSED_SIZE];
            g1.copy_from_slice(&buf[start..start + G1_COMPRESSED_SIZE]);
            g2.copy_from_slice(
                &buf[start + G1_COMPRESSED_SIZE..start + G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE],
            );
            pairs.push((g1, g2));
        }

        let mut out = [0u8; GT_COMPRESSED_SIZE];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(
        pairs: &[([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])],
        out: &mut [u8; GT_COMPRESSED_SIZE],
    ) {
        let mut acc: Option<Gt> = None;
        for (g1b, g2b) in pairs.iter() {
            let g1 = G1Affine::from_compressed(g1b);
            let g2 = G2Affine::from_compressed(g2b);
            if g1.is_none().unwrap_u8() == 1 || g2.is_none().unwrap_u8() == 1 {
                out.fill(0);
                return;
            }
            let e = pairing(&g1.unwrap(), &g2.unwrap());
            acc = Some(match acc {
                Some(a) => a + e,
                None => e,
            });
            e.compress().unwrap();
        }
        let res = acc.unwrap_or_else(Gt::identity);
        // Write compressed GT (288 bytes) into the output buffer; remaining bytes stay zeroed
        let mut cursor = std::io::Cursor::new(&mut out[..]);
        res.write_compressed(&mut cursor).unwrap();
    }
}
