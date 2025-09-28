// use crate::{
//     syscall_handler::bls12381::{parse_affine_g1, G1_UNCOMPRESSED_LENGTH},
//     RuntimeContext,
// };
// use blstrs::{G1Affine, G1Projective};
// use rwasm::{Store, TrapCode, TypedCaller, Value};

// pub struct SyscallBls12381G1Add;

// impl SyscallBls12381G1Add {
//     pub fn fn_handler(
//         caller: &mut TypedCaller<RuntimeContext>,
//         params: &[Value],
//         _result: &mut [Value],
//     ) -> Result<(), TrapCode> {
//         let p_ptr = params[0].i32().unwrap() as usize;
//         let q_ptr = params[1].i32().unwrap() as usize;

//         let mut p = [0u8; G1_UNCOMPRESSED_LENGTH];
//         caller.memory_read(p_ptr, &mut p)?;

//         let mut q = [0u8; G1_UNCOMPRESSED_LENGTH];
//         caller.memory_read(q_ptr, &mut q)?;

//         Self::fn_impl(&mut p, &q);
//         caller.memory_write(p_ptr, &p)?;

//         Ok(())
//     }

//     pub fn fn_impl(p: &mut [u8; G1_UNCOMPRESSED_LENGTH], q: &[u8; G1_UNCOMPRESSED_LENGTH]) {
//         let p_aff = parse_affine_g1(p);
//         let q_aff = parse_affine_g1(q);

//         let result_proj = G1Projective::from(p_aff) + G1Projective::from(q_aff);
//         p.copy_from_slice(&G1Affine::from(result_proj).to_uncompressed());
//     }
// }
