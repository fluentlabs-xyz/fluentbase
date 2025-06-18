use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{Caller, TrapCode};

pub const SHA_COMPRESS_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub(crate) struct SyscallSha256Compress;

impl SyscallSha256Compress {
    #[allow(clippy::many_single_char_names)]
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let (w_ptr, h_ptr) = caller.stack_pop2_as::<u32>();
        if w_ptr == h_ptr {
            caller.context_mut().execution_result.exit_code =
                ExitCode::MalformedBuiltinParams.into_i32();
            return Err(TrapCode::ExecutionHalted);
        }

        // Execute the "initialize" phase where we read in the h values.
        let mut hx = [0u32; 8];
        for i in 0..8 {
            let value = u32::from_be_bytes(
                caller
                    .memory_read_fixed::<4>((h_ptr + i as u32 * 4) as usize)?
                    .try_into()
                    .unwrap(),
            );
            hx[i] = value;
        }

        let mut original_w = Vec::new();
        // Execute the "compress" phase.
        let mut a = hx[0];
        let mut b = hx[1];
        let mut c = hx[2];
        let mut d = hx[3];
        let mut e = hx[4];
        let mut f = hx[5];
        let mut g = hx[6];
        let mut h = hx[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let w_i = u32::from_be_bytes(
                caller
                    .memory_read_fixed::<4>((w_ptr + i * 4) as usize)?
                    .try_into()
                    .unwrap(),
            );
            original_w.push(w_i);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA_COMPRESS_K[i as usize])
                .wrapping_add(w_i);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        // Execute the "finalize" phase.
        let v = [a, b, c, d, e, f, g, h];
        for i in 0..8 {
            caller.memory_write(
                (h_ptr + i as u32 * 4) as usize,
                &hx[i].wrapping_add(v[i]).to_be_bytes(),
            )?;
        }

        Ok(())
    }

    pub fn fn_impl(_w: &[u8], _h: &[u8]) -> Result<(), TrapCode> {
        todo!("not implemented yet")
        // if w.as_ptr() == h.as_ptr() {
        //     return Err(ExitCode::BadBuiltinParams.into_trap());
        // }
        //
        // // Execute the "initialize" phase where we read in the h values.
        // let mut hx = [0u32; 8];
        // for i in 0..8 {
        //     let value = u32::from_be_bytes(
        //         caller
        //             .read_memory(h_ptr + i as u32 * 4, 4)?
        //             .try_into()
        //             .unwrap(),
        //     );
        //     hx[i] = value;
        // }
        //
        // let mut original_w = Vec::new();
        // // Execute the "compress" phase.
        // let mut a = hx[0];
        // let mut b = hx[1];
        // let mut c = hx[2];
        // let mut d = hx[3];
        // let mut e = hx[4];
        // let mut f = hx[5];
        // let mut g = hx[6];
        // let mut h = hx[7];
        // for i in 0..64 {
        //     let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        //     let ch = (e & f) ^ (!e & g);
        //     let w_i = u32::from_be_bytes(caller.read_memory(w_ptr + i * 4,
        // 4)?.try_into().unwrap());     original_w.push(w_i);
        //     let temp1 = h
        //         .wrapping_add(s1)
        //         .wrapping_add(ch)
        //         .wrapping_add(SHA_COMPRESS_K[i as usize])
        //         .wrapping_add(w_i);
        //     let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        //     let maj = (a & b) ^ (a & c) ^ (b & c);
        //     let temp2 = s0.wrapping_add(maj);
        //
        //     h = g;
        //     g = f;
        //     f = e;
        //     e = d.wrapping_add(temp1);
        //     d = c;
        //     c = b;
        //     b = a;
        //     a = temp1.wrapping_add(temp2);
        // }
        //
        // // Execute the "finalize" phase.
        // let v = [a, b, c, d, e, f, g, h];
        // for i in 0..8 {
        //     caller.write_memory(
        //         h_ptr + i as u32 * 4,
        //         &hx[i].wrapping_add(v[i]).to_be_bytes(),
        //     )?;
        // }
        //
        // Ok(())
    }
}
