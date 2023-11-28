#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{EQ, GT, LT, PUSH0, PUSH32, SHL},
        },
    };
    use alloy_primitives::{hex, Bytes};
    use fluentbase_runtime::Runtime;
    use fluentbase_rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, ReducedModule};
    fn d(hex: &str) -> Vec<u8> {
        hex::decode(hex.replace(" ", "")).unwrap()
    }

    use crate::translator::instructions::opcode::{BYTE, SHR, SUB};
    use log::debug;

    fn run_test(evm_bytecode_bytes: &Vec<u8>) -> [u8; 32] {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let mut compiler = EvmCompiler::new(evm_binary.as_ref(), false);

        compiler.instruction_set.op_i32_const(100);
        compiler.instruction_set.op_memory_grow();
        compiler.instruction_set.op_drop();

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);

        compiler
            .instruction_set
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let rwasm_binary = binary_format_writer.to_vec();

        let mut instruction_set = ReducedModule::new(&rwasm_binary, true)
            .unwrap()
            .bytecode()
            .clone();
        debug!(
            "\nrmodule.trace_binary() (rwasm_binary.len={}): \n{}\n",
            rwasm_binary.len(),
            instruction_set.trace()
        );

        let mut global_memory = vec![0u8; 4086];
        let mut global_memory_len: usize = 0;
        let result = Runtime::run(&rwasm_binary, &Vec::new(), 0);
        assert!(result.is_ok());
        let execution_result = result.unwrap();
        // debug!("mem changes:");
        for log in execution_result.tracer().logs.iter() {
            if log.memory_changes.len() > 0 {
                debug!(
                    "log opcode {} memory_changes {:?}",
                    log.opcode, &log.memory_changes
                );
            }
            for change in &log.memory_changes {
                let new_len = (change.offset + change.len) as usize;
                global_memory[change.offset as usize..new_len].copy_from_slice(&change.data);
                if new_len > global_memory_len {
                    global_memory_len = (change.offset + change.len) as usize;
                }
            }
        }
        assert!(global_memory_len <= 32);
        let global_memory = global_memory[0..32].to_vec();
        debug!(
            "global_memory (len {}) {:?}",
            global_memory_len,
            &global_memory[..global_memory_len]
        );
        // debug!(
        //     "\nexecution_result.tracer() (exit_code {}): \n{:#?}\n",
        //     execution_result.data().exit_code(),
        //     execution_result.tracer()
        // );
        assert_eq!(execution_result.data().exit_code(), 0);

        global_memory.try_into().unwrap()
    }

    fn test_binary(opcode: u8, cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)]) {
        assert!(cases.len() > 0);
        for case in cases {
            let a = &case.0;
            let b = &case.1;
            let res_expected = &case.2;
            let mut evm_bytecode_bytes: Vec<u8> = vec![];
            // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
            evm_bytecode_bytes.push(PUSH0);
            evm_bytecode_bytes.push(PUSH32);
            evm_bytecode_bytes.extend(a);
            evm_bytecode_bytes.push(PUSH32);
            evm_bytecode_bytes.extend(b);
            evm_bytecode_bytes.push(opcode);

            let mut global_memory = run_test(&evm_bytecode_bytes);
            const CHUNK_LEN: usize = 8;
            for chunk in global_memory.chunks_mut(8) {
                for i in 0..(CHUNK_LEN / 2) {
                    let tmp = chunk[i];
                    let opposite_index = CHUNK_LEN - 1 - i;
                    chunk[i] = chunk[opposite_index];
                    chunk[opposite_index] = tmp;
                }
            }
            if res_expected[..] != global_memory[..] {
                debug!("a=           {:x?}", a);
                debug!("b=            {:x?}", b);
                debug!("res_expected= {:x?}", res_expected);
                debug!("res=          {:x?}", global_memory);
            }
            assert_eq!(res_expected[..], global_memory);
        }
    }

    #[test]
    fn eq() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-4 b=-5 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-5 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30000 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30000 b=-30001 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30001 b=-30000 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=1 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=2 b=1 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=2 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30000 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30001 b=30000 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30001 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary(EQ, &cases);
    }

    #[test]
    fn shl() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=4
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // shift=2 value=1 r=4
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                d("0xF000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFE"),
                d("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x8000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000100"),
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0xF000000000000000000000000000000000000000000000000000000000000000"),
                d("0xe000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary(SHL, &cases);
    }

    #[test]
    fn shr() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // shift=2 value=4 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // shift=1 value=4 r=2
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x00000000000000000000000000000000000000000000000000000000000000FF"),
                d("0x000000000000000000000000000000000000000000000000000000000000000F"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000008"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            // externally checked
            (
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000100"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000101"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0x0000000000000000000000000F00000000000000000000000000000000000000"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                d("0xF000000000000000000000000000000000000000000000000000000000000000"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary(SHR, &cases);
    }

    #[test]
    fn byte() {
        // [(idx, value, r), ...]
        let mut cases = vec![
            // shift=32 value=0xff..ff r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000020"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // shift=33 value=0xff..ff r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000021"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        for i in 0..32 {
            let mut idx = d("0x0000000000000000000000000000000000000000000000000000000000000000");
            let mut res = d("0x0000000000000000000000000000000000000000000000000000000000000000");
            let last_byte_idx = idx.len() - 1;
            idx[last_byte_idx] = i;
            res[last_byte_idx] = i + 1;
            cases.push((
                idx,
                d("0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                res,
            ));
        }

        test_binary(BYTE, &cases);
    }

    #[test]
    fn slt() {
        // TODO
    }

    #[test]
    fn sgt() {
        // TODO
    }

    #[test]
    fn sar() {
        // TODO
    }

    #[test]
    fn sub() {
        let cases = [
            // // cases where: a>=0, b>=0, a-b >=0
            // // a=1 b=1 r=0
            // (
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
            // // a=2 b=1 r=0
            // (
            //     d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            // ),
            // // a=3 b=1 r=0
            // (
            //     d("0x0000000000000000000000000000000000000000000000000000000000000003"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            // ),
            // // a=30001 b=1 r=30000
            // (
            //     d("0x0000000000000000000000000000000000000000000000000000000000007531"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000007530"),
            // ),
            // // a=b b=a r=0
            // (
            //     d("0x0000000012000000012340000000f0000020000000f123000000030000000001"),
            //     d("0x0000000012000000012340000000f0000020000000f123000000030000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
            // // externally checked
            // (
            //     d("0x000000000000000000000000000000000000000000f123000000000000000001"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000007531"),
            //     d("0x000000000000000000000000000000000000000000f122ffffffffffffff8ad0"),
            // ),
            // // externally checked
            // (
            //     d("0x000000010000001000111111f00010000200030000f123000000000000000001"),
            //     d("0x0000000000000010001000000000002000000003000000000000000000007531"),
            //     d("0x000000010000000000011111f0000fe0020002fd00f122ffffffffffffff8ad0"),
            // ),
            // // externally checked (bug?)
            // (
            //     d("0x000000000000000f00f0010000000000 00000000000000000000000000000000"),
            //     d("0x000000000000000000f0020000000000 00000000000000000000000000000000"),
            //     d("0x000000000000000effffff0000000000 00000000000000000000000000000000"),
            // ),
            // externally checked (bug?)
            (
                d("0x 000000000000000f0000000000000000 00000000000000000000000000000000"),
                d("0x 00000000000000000000000000000001 00000000000000000000000000000000"),
                d("0x 000000000000000effffffffffffffff 00000000000000000000000000000000"),
            ),
            // // externally checked (no bug)
            // (
            //     d("0x 00000000000000000000000000000000 000000000000000f0000000000000000"),
            //     d("0x 00000000000000000000000000000000 00000000000000000000000000000001"),
            //     d("0x 00000000000000000000000000000000 000000000000000effffffffffffffff"),
            // ),
            // externally checked
            // (
            //     d("0x000000000f00000f00000000f00010000200030000f123000000000000000001"),
            //     d("0x000000000000002000f002000000002000000003000000000000000000007531"),
            //     d("0x000000000effffeeff0ffe00f0000fe0020002fd00f122ffffffffffffff8ad0"),
            // ),
            // a=0 b=9 r=-9
            // (
            //     d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     d("0x0000000000000000000000000000000000000000000000000000000000000009"),
            //     d("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7"),
            // ),
            // // a=770000000000000000000000000000000000000000000
            // // b=3000000000000000000000000000000000000000
            // // r=769997000000000000000000000000000000000000000
            // (
            //     d("0x0000000000000000000000000022872aa015d1317152fc14d55fc80000000000"),
            //     d("0x00000000000000000000000000000008d0f2fbba9bcb064d1e30038000000000"),
            //     d("0x00000000000000000000000000228721cf22d576d587f5c7b72fc48000000000"),
            // ),
        ];

        test_binary(SUB, &cases);
    }

    #[test]
    fn gt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-4 b=-5 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-5 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30000 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30001 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30001 b=-30000 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=1 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=2 b=1 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=1 b=2 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30000 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30001 b=30000 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30000 b=30001 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary(GT, &cases);
    }

    #[test]
    fn lt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-4 b=-5 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-5 b=-4 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30000 b=-30000 r=0
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30001 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30001 b=-30000 r=1
            (
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=1 b=1 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=2 b=1 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=2 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30000 b=30000 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30001 b=30000 r=1
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30001 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_binary(LT, &cases);
    }
}
