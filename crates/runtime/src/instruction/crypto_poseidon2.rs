use crate::RuntimeContext;
//use fluentbase_poseidon::Hashable;
use fluentbase_types::ExitCode;
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use rwasm::{common::Trap, Caller};
use fluentbase_poseidon::hash_msg_with_domain;

pub struct CryptoPoseidon2;

impl CryptoPoseidon2 {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        fa_offset: u32,
        fb_offset: u32,
        fd_offset: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        println!("DEBUG POSEIDON 2 WE CALL fn handler");
        let output = Self::fn_impl(
            caller.read_memory(fa_offset, 32),
            caller.read_memory(fb_offset, 32),
            caller.read_memory(fd_offset, 32),
        )?;
        // TODO: here trap is not raised later and on error we just not getting this memory write, so it must be raised
        println!("DEBUG WE DO MEMORY WRITE FOR POSEIDON 2 fn handler");
        caller.write_memory(output_offset, &output);
        Ok(())
    }

    pub fn fn_impl(fa: &[u8], fb: &[u8], fd: &[u8]) -> Result<[u8; 32], Trap> {
        println!("DEBUG POSEIDON 2 WE CALL fn impl");
        let fr_from_bytes = |fr_data: &[u8]| -> Result<Fr, Trap> {
            let fr_data: [u8; 32] = fr_data
                .try_into()
                .map_err(|_| <ExitCode as Into<Trap>>::into(ExitCode::PoseidonError))?;
            let fr = Fr::from_bytes(&fr_data);
            if fr.is_none().into() {
                return Err(<ExitCode as Into<Trap>>::into(ExitCode::PoseidonError));
            }
            Ok(fr.unwrap())
        };

        // TODO: find more simple solution
        let mut draft_fa = [0_u8; 32];
        draft_fa.copy_from_slice(fa.iter().rev().map(|x| *x).collect::<Vec<u8>>().as_slice());
        let mut draft_fb = [0_u8; 32];
        draft_fb.copy_from_slice(fb.iter().rev().map(|x| *x).collect::<Vec<u8>>().as_slice());
        let mut draft_fd = [0_u8; 32];
        draft_fd.copy_from_slice(fd.iter().rev().map(|x| *x).collect::<Vec<u8>>().as_slice());
 
        println!("DEBUG POSEIDON 2 POINT A");
        let fa = fr_from_bytes(&draft_fa)?;
        println!("DEBUG POSEIDON 2 POINT B");
        let fb = fr_from_bytes(&draft_fb)?;
        println!("DEBUG POSEIDON 2 POINT D");
        let fd = fr_from_bytes(&draft_fd)?;
        // TODO: example error is failing with this fr transformation from bytes
        println!("DEBUG POSEIDON 2 WE GETTING INPUT {:#?} {:#?} {:#?}", &fa, &fb, &fd);
        //let hasher = Fr::hasher();
        //let h2 = hasher.hash([fa, fb], fd);
        let h2 = hash_msg_with_domain(&[fa, fb], fd);
        println!("DEBUG POSEIDON 2 WE COMPUTING RESULT {:#?}", &h2);
        Ok(h2.to_repr())
    }
}
