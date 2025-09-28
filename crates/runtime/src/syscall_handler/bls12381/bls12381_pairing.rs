// use crate::RuntimeContext;
// use blstrs::Compress as _;
// use blstrs::{pairing, G1Affine, G2Affine, Gt};
// use group::Group;
// use rwasm::{Store, TrapCode, TypedCaller, Value};

// const G1_COMPRESSED_SIZE: usize = 48;
// const G2_COMPRESSED_SIZE: usize = 96;
// const GT_COMPRESSED_SIZE: usize = 288;

// pub struct SyscallBls12381Pairing;

// /// Expects `pairs_len` pairs laid out in memory as contiguous chunks where each pair is:
// ///  - 48 bytes: compressed G1 point (blstrs format)
// ///  - 96 bytes: compressed G2 point (blstrs format)
// ///
// /// Writes a 288-byte compressed GT element (blstrs format) to `out_ptr` that
// /// represents the product of pairings over all provided pairs. If any input
// /// point fails to decompress/validate, the output buffer is filled with zeros.
// impl SyscallBls12381Pairing {
//     pub fn fn_handler(
//         caller: &mut TypedCaller<RuntimeContext>,
//         params: &[Value],
//         _result: &mut [Value],
//     ) -> Result<(), TrapCode> {
//         let pairs_ptr = params[0].i32().unwrap() as usize;
//         let pairs_len = params[1].i32().unwrap() as usize; // number of pairs
//         let out_ptr = params[2].i32().unwrap() as usize;

//         // Each pair is 144 bytes: 48 bytes G1 + 96 bytes G2 (blstrs compressed)
//         let total_len = pairs_len * (G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE);
//         let mut buf = vec![0u8; total_len];
//         caller.memory_read(pairs_ptr, &mut buf)?;

//         // Parse into vector of pairs
//         let mut pairs: Vec<([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])> =
//             Vec::with_capacity(pairs_len);
//         for i in 0..pairs_len {
//             let start = i * (G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE);
//             let mut g1 = [0u8; G1_COMPRESSED_SIZE];
//             let mut g2 = [0u8; G2_COMPRESSED_SIZE];
//             g1.copy_from_slice(&buf[start..start + G1_COMPRESSED_SIZE]);
//             g2.copy_from_slice(
//                 &buf[start + G1_COMPRESSED_SIZE..start + G1_COMPRESSED_SIZE + G2_COMPRESSED_SIZE],
//             );
//             pairs.push((g1, g2));
//         }

//         let mut out = [0u8; GT_COMPRESSED_SIZE];
//         Self::fn_impl(&pairs, &mut out);
//         caller.memory_write(out_ptr, &out)?;
//         Ok(())
//     }

//     pub fn fn_impl(
//         pairs: &[([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])],
//         out: &mut [u8; GT_COMPRESSED_SIZE],
//     ) {
//         let mut acc: Option<Gt> = None;
//         for (g1b, g2b) in pairs.iter() {
//             let g1 = G1Affine::from_compressed(g1b);
//             let g2 = G2Affine::from_compressed(g2b);
//             if g1.is_none().unwrap_u8() == 1 || g2.is_none().unwrap_u8() == 1 {
//                 out.fill(0);
//                 return;
//             }
//             let e = pairing(&g1.unwrap(), &g2.unwrap());
//             acc = Some(match acc {
//                 Some(a) => a + e,
//                 None => e,
//             });
//         }
//         let res = acc.unwrap_or_else(Gt::identity);
//         // For compatibility with the contract which checks for zeroed buffer to determine identity,
//         // write zeros when the accumulated pairing is the multiplicative identity.
//         if res == Gt::identity() {
//             out.fill(0);
//         } else {
//             // Write compressed GT (288 bytes) into the output buffer
//             let mut cursor = std::io::Cursor::new(&mut out[..]);
//             res.write_compressed(&mut cursor).unwrap();
//         }
//     }
// }
