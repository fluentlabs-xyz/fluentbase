// use crate::{
//     syscall_handler::bls12381::{
//         bls12381_consts::G2_UNCOMPRESSED_LENGTH,
//         bls12381_helpers::{
//             g2_be_uncompressed_to_le_limbs, g2_le_limbs_to_be_uncompressed, parse_affine_g2,
//         },
//     },
//     RuntimeContext,
// };
// use blstrs::{G2Affine, G2Projective};
// use rwasm::{Store, TrapCode, TypedCaller, Value};

// pub struct SyscallBls12381G2Add;

// impl SyscallBls12381G2Add {
//     pub fn fn_handler(
//         caller: &mut TypedCaller<RuntimeContext>,
//         params: &[Value],
//         _result: &mut [Value],
//     ) -> Result<(), TrapCode> {
//         let p_ptr = params[0].i32().unwrap() as usize;
//         let q_ptr = params[1].i32().unwrap() as usize;

//         let mut p = [0u8; G2_UNCOMPRESSED_LENGTH];
//         caller.memory_read(p_ptr, &mut p)?;

//         let mut q = [0u8; G2_UNCOMPRESSED_LENGTH];
//         caller.memory_read(q_ptr, &mut q)?;

//         Self::fn_impl(&mut p, &q);
//         caller.memory_write(p_ptr, &p)?;
//         Ok(())
//     }

//     pub fn fn_impl(p: &mut [u8; G2_UNCOMPRESSED_LENGTH], q: &[u8; G2_UNCOMPRESSED_LENGTH]) {
//         // p, q layout: x0||x1||y0||y1, each limb 48 bytes little-endian
//         // Convert to blstrs uncompressed big-endian bytes with c0/c1 swapped, add, then convert back.
//         let a_be = g2_le_limbs_to_be_uncompressed(p);
//         let b_be = g2_le_limbs_to_be_uncompressed(q);

//         let a_aff = parse_affine_g2(&a_be);
//         let b_aff = parse_affine_g2(&b_be);

//         let sum = G2Projective::from(a_aff) + G2Projective::from(b_aff);
//         let sum_aff = G2Affine::from(sum);

//         // Serialize to BE uncompressed and convert back to LE limb format
//         p.copy_from_slice(&g2_be_uncompressed_to_le_limbs(&sum_aff.to_uncompressed()));
//     }
// }
