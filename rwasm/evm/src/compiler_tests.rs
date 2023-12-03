#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{
                BYTE,
                EQ,
                GT,
                KECCAK256,
                LT,
                MSTORE,
                MSTORE8,
                PUSH32,
                SHL,
                SHR,
                SUB,
            },
        },
    };
    use alloy_primitives::{hex, Bytes};
    use fluentbase_runtime::{ExecutionResult, Runtime};
    use fluentbase_rwasm::{
        engine::bytecode::Instruction,
        rwasm::{BinaryFormat, BinaryFormatWriter, ReducedModule},
    };
    use log::debug;

    fn d(hex: &str) -> Vec<u8> {
        let mut res = hex.replace(" ", "");
        if !res.starts_with("0x") {
            res = "0x".to_string();
            res.push_str(hex);
        }
        hex::decode(res.clone()).unwrap()
    }

    fn compile_binary_op(
        opcode: u8,
        push_offset_where_to_save_result_in_memory: bool,
        a: &[u8],
        b: &[u8],
    ) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        // if push_offset_for_result_in_memory {
        //     evm_bytecode.push(PUSH0);
        // }
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(a);
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(b);
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    /// @cases - &(a,b,result)
    fn test_binary_op(
        opcode: u8,
        initial_bytecode: Option<&[u8]>,
        push_offset_where_to_save_result_in_memory: bool,
        cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)],
        force_memory_result_size_to: Option<usize>,
    ) {
        assert!(cases.len() > 0);
        for case in cases {
            let a = &case.0;
            let b = &case.1;
            let res_expected = &case.2;
            let mut evm_bytecode: Vec<u8> = vec![];
            initial_bytecode.map(|v| evm_bytecode.extend(v));
            // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
            evm_bytecode.extend(compile_binary_op(
                opcode,
                push_offset_where_to_save_result_in_memory,
                a,
                b,
            ));

            let mut global_memory = run_test(&evm_bytecode, force_memory_result_size_to);
            let res = &global_memory[0..res_expected.len()];
            if res_expected != res {
                debug!("a=            {:x?}", a);
                debug!("b=            {:x?}", b);
                debug!("res_expected= {:x?}", res_expected);
                debug!("res=          {:x?}", global_memory);
            }
            assert_eq!(res_expected, res);
        }
    }

    fn run_test(
        evm_bytecode_bytes: &Vec<u8>,
        force_memory_result_size_to: Option<usize>,
    ) -> Vec<u8> {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let import_linker = Runtime::<()>::new_linker();
        let mut compiler = EvmCompiler::new(&import_linker, false, evm_binary.as_ref());

        let res = compiler.compile();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);

        compiler
            .instruction_set
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let rwasm_binary = binary_format_writer.to_vec();

        let mut instruction_set = ReducedModule::new(&rwasm_binary, false)
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
        let execution_result: ExecutionResult<()> = result.unwrap();
        for (index, log) in execution_result.tracer().logs.iter().enumerate() {
            if log.memory_changes.len() <= 0 {
                continue;
            };
            let prev_opcode = if index > 0 {
                Some(execution_result.tracer().logs[index - 1].opcode)
            } else {
                None
            };
            let mut memory_changes = log.memory_changes.clone();
            match prev_opcode {
                Some(Instruction::I64Store(_)) => {
                    for change in &mut memory_changes {
                        let v = i64::from_le_bytes(change.data.as_slice().try_into().unwrap());
                        change.data.clone_from_slice(v.to_be_bytes().as_slice());
                    }
                }
                _ => {}
            }
            debug!("opcode:{} memory_changes:{:?}", log.opcode, &memory_changes);
            for change in &memory_changes {
                let new_len = (change.offset + change.len) as usize;
                global_memory[change.offset as usize..new_len].copy_from_slice(&change.data);
                if new_len > global_memory_len {
                    global_memory_len = (change.offset + change.len) as usize;
                }
            }
        }
        let global_memory = global_memory[0..force_memory_result_size_to.unwrap_or(32)].to_vec();
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

        global_memory
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

        test_binary_op(EQ, None, true, &cases, None);
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

        test_binary_op(SHL, None, true, &cases, None);
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

        test_binary_op(SHR, None, true, &cases, None);
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

        // test-cases for all possible byte values
        let start = 0;
        let len = 32;
        for i in start..start + len {
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

        test_binary_op(BYTE, None, true, &cases, None);
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
            // cases where: a>=0, b>=0, a-b >=0
            // a=1 b=1 r=0
            (
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
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
            // // externally checked (bug?)
            // (
            //     d("0x 000000000000000f0000000000000000 00000000000000000000000000000000"),
            //     d("0x 00000000000000000000000000000001 00000000000000000000000000000000"),
            //     d("0x 000000000000000effffffffffffffff 00000000000000000000000000000000"),
            // ),
            // // externally checked (no bug)
            // (
            //     d("0x 00000000000000000000000000000000 000000000000000f0000000000000000"),
            //     d("0x 00000000000000000000000000000000 00000000000000000000000000000001"),
            //     d("0x 00000000000000000000000000000000 000000000000000effffffffffffffff"),
            // ),
            // // externally checked
            // (
            //     d("0x000000000f00000f00000000f00010000200030000f123000000000000000001"),
            //     d("0x000000000000002000f002000000002000000003000000000000000000007531"),
            //     d("0x000000000effffeeff0ffe00f0000fe0020002fd00f122ffffffffffffff8ad0"),
            // ),
            // // a=0 b=9 r=-9
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

        test_binary_op(SUB, None, true, &cases, None);
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

        test_binary_op(GT, None, true, &cases, None);
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

        test_binary_op(LT, None, true, &cases, None);
    }

    #[test]
    fn mstore() {
        let max_result_size = 8 + 32; // multiple of 8
        let max_result_size = (max_result_size + 7) / 8 * 8;
        // expected results must be multiple of 8 bytes (i64/u64 value) or memory swap will spoil it
        let cases = [
            // offset=0 value= r=0
            (
                d("0000000000000000000000000000000000000000000000000000000000000000"),
                d("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                d("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            ),
            // offset=8 value= r=0
            (
                d("0000000000000000000000000000000000000000000000000000000000000008"),
                d("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                d("0000000000000000000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            ),
        ];

        test_binary_op(MSTORE, None, false, &cases, Some(max_result_size));
    }

    #[test]
    fn mstore8() {
        let max_result_size = 8 + 32; // multiple of 8
        let max_result_size = (max_result_size + 7) / 8 * 8;
        // expected results must be multiple of 8 bytes (i64/u64 value) or memory swap will spoil it
        let cases = [
            // offset=0 value= r=0
            (
                d("0000000000000000000000000000000000000000000000000000000000000000"),
                d("000000000f00000100000000f0000000100000000f00000000000000000f0032"),
                d("3200000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                d("0000000000000000000000000000000000000000000000000000000000000008"),
                d("000000000f00000100000000f0000000100000000f00000000000000000f00af"),
                d("0000000000000000af0000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary_op(MSTORE8, None, false, &cases, Some(max_result_size));
    }

    #[test]
    fn keccak256() {
        let max_result_size = 32; // multiple of 8
        let max_result_size = (max_result_size + 7) / 8 * 8;
        // [(initial_bytecode, (a,b,result)), ...]
        let cases = [(
            compile_binary_op(
                MSTORE,
                false,
                &d("0000000000000000000000000000000000000000000000000000000000000000"),
                &d("FFFFFFFF00000000000000000000000000000000000000000000000000000000"),
            ),
            (
                d("0000000000000000000000000000000000000000000000000000000000000000"),
                d("0000000000000000000000000000000000000000000000000000000000000004"),
                d("29045a592007d0c246ef02c2223570da9522d0cf0f73282c79a1bc8f0bb2c238"),
            ),
        )];
        for case in &cases {
            test_binary_op(
                KECCAK256,
                Some(&case.0),
                true,
                &[case.1.clone()],
                Some(max_result_size),
            );
        }
    }
}
