#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{
                ADD,
                BYTE,
                EQ,
                GT,
                KECCAK256,
                LT,
                MSTORE,
                MSTORE8,
                MUL,
                PUSH32,
                SHL,
                SHR,
                SUB,
            },
        },
        utilities::EVM_WORD_BYTES,
    };
    use alloy_primitives::{hex, Bytes};
    use fluentbase_runtime::{ExecutionResult, Runtime};
    use fluentbase_rwasm::{
        engine::bytecode::Instruction,
        rwasm::{BinaryFormat, BinaryFormatWriter, InstructionSet, ReducedModule},
    };
    use log::debug;

    fn x(hex: &str) -> Vec<u8> {
        let mut h = hex.replace(" ", "");
        if !h.starts_with("0x") {
            h = "0x".to_string();
            h.push_str(hex);
        }
        let res = hex::decode(h.clone());
        if let Err(v) = res {
            panic!("failed to decode hex value '{:?}'", hex);
        }
        res.unwrap()
    }

    fn xr(hex: &str, le_from_offset: usize) -> Vec<u8> {
        let mut r = x(hex);
        const SIZE: usize = EVM_WORD_BYTES;
        r[le_from_offset..].chunks_mut(SIZE).for_each(|chunk| {
            if chunk.len() != EVM_WORD_BYTES {
                return;
            };
            for i in 0..SIZE / 2 {
                let tmp = chunk[i];
                chunk[i] = chunk[SIZE - i - 1];
                chunk[SIZE - i - 1] = tmp;
            }
        });
        r
    }

    fn compile_binary_op(opcode: u8, a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(b);
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(a);
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    /// @cases - &(a,b,result)
    fn test_binary_op(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)],
        force_memory_result_size_to: Option<usize>,
    ) {
        assert!(cases.len() > 0);
        for case in cases {
            let a = &case.0;
            let b = &case.1;
            let mut res_expected = case.2.clone();
            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));
            // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
            evm_bytecode.extend(compile_binary_op(opcode, a, b));

            let mut global_memory = run_test(&evm_bytecode, force_memory_result_size_to);
            let res = &global_memory[0..res_expected.len()];
            if res_expected != res {
                debug!("a=            {:?}", a);
                debug!("b=            {:?}", b);
                debug!("res_expected= {:?}", res_expected);
                debug!("res=          {:?}", global_memory);
            }
            assert_eq!(res_expected, res);
        }
    }

    fn run_test(evm_bytecode_bytes: &Vec<u8>, force_memory_result_size: Option<usize>) -> Vec<u8> {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let import_linker = Runtime::<()>::new_linker();
        let mut compiler = EvmCompiler::new(&import_linker, false, evm_binary.as_ref());

        let mut preamble = InstructionSet::new();
        let virtual_stack_top = 300;
        preamble.op_i64_const(virtual_stack_top); // virtual stack top offset
        preamble.op_global_set(0);
        preamble.op_i32_const(20);
        preamble.op_memory_grow();
        preamble.op_drop();
        let res = compiler.compile(Some(&preamble), None);
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);

        compiler
            .instruction_set
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let rwasm_binary = binary_format_writer.to_vec();

        let mut rmodule = ReducedModule::new(&rwasm_binary, false).unwrap();
        let mut instruction_set = rmodule.bytecode().clone();
        debug!(
            "\nrmodule.trace_binary() (rwasm_binary.len={}): \n{}\n",
            rwasm_binary.len(),
            instruction_set.trace()
        );

        let mut global_memory = vec![0u8; virtual_stack_top];
        let mut global_memory_len: usize = 0;
        let result = Runtime::run(&rwasm_binary, &Vec::new(), 0);
        assert!(result.is_ok());
        let execution_result: ExecutionResult<()> = result.unwrap();
        for (index, log) in execution_result.tracer().logs.iter().enumerate() {
            if log.memory_changes.len() <= 0 {
                continue;
            };
            let memory_changes = log.memory_changes.clone();
            // let prev_opcode = if index > 0 {
            //     Some(execution_result.tracer().logs[index - 1].opcode)
            // } else {
            //     None
            // };
            // match prev_opcode {
            //     Some(Instruction::I64Store(_)) => {
            //         for change in &mut memory_changes {
            //             let v = i64::from_le_bytes(change.data.as_slice().try_into().unwrap());
            //             change.data.clone_from_slice(v.to_be_bytes().as_slice());
            //         }
            //     }
            //     _ => {}
            // }
            debug!(
                "opcode:{:x?} memory_changes:{:?}",
                log.opcode, &memory_changes
            );
            for change in &memory_changes {
                let offset_start = change.offset as usize;
                let offset_end = offset_start + change.len as usize;
                global_memory[offset_start..offset_end].copy_from_slice(&change.data);
                if offset_end > global_memory_len {
                    global_memory_len = offset_end;
                }
            }
        }
        debug!(
            "global_memory part (total len {}) {:?}",
            global_memory_len, &global_memory
        );
        let global_memory =
            global_memory[0..force_memory_result_size.unwrap_or(EVM_WORD_BYTES)].to_vec();
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
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(EQ, None, &cases, None);
    }

    #[test]
    fn shl() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=4
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // shift=2 value=1 r=4
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000004",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0xF000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFE"),
                x("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                xr(
                    "0x8000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xF000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0xe000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SHL, None, &cases, None);
    }

    #[test]
    fn shr() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // shift=2 value=4 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // shift=1 value=4 r=2
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x00000000000000000000000000000000000000000000000000000000000000FF"),
                xr(
                    "0x000000000000000000000000000000000000000000000000000000000000000F",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000101"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000F00000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // externally checked
            (
                x("0xF000000000000000000000000000000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SHR, None, &cases, None);
    }

    #[test]
    fn byte() {
        // [(idx, value, r), ...]
        let mut cases = vec![
            // shift=32 value=0xff..ff r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000020"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // shift=33 value=0xff..ff r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000021"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        // test-cases for all possible byte values
        let start = 0;
        let len = 32;
        for i in start..start + len {
            let mut idx = x("0x0000000000000000000000000000000000000000000000000000000000000000");
            let mut res = x("0x0000000000000000000000000000000000000000000000000000000000000000");
            let last_byte_idx = idx.len() - 1;
            idx[last_byte_idx] = i;
            res[0] = i + 1; // because LE
            cases.push((
                idx,
                x("0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                res,
            ));
        }

        test_binary_op(BYTE, None, &cases, None);
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
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=2 b=1 r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=3 b=1 r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // a=30001 b=1 r=30000
            (
                x("0000000000000000000000000000000000000000000000000000000000007531"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000007530",
                    0,
                ),
            ),
            // a=b b=a r=0
            (
                x("0000000012000000012340000000f0000020000000f123000000030000000001"),
                x("0000000012000000012340000000f0000020000000f123000000030000000001"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=291516405354026068686667777 b=30001 r=291516405354026068686637776
            (
                x("000000000000000000000000000000000000000000f123000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000007531"),
                xr(
                    "000000000000000000000000000000000000000000f122ffffffffffffff8ad0",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("000000010000001000111111f00010000200030000f123000000000000000001"),
                x("0000000000000010001000000000002000000003000000000000000000007531"),
                xr(
                    "000000010000000000011111f0000fe0020002fd00f122ffffffffffffff8ad0",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("000000000000000f00f001000000000000000000000000000000000000000000"),
                x("000000000000000000f002000000000000000000000000000000000000000000"),
                xr(
                    "000000000000000effffff000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("000000000000000f000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000100000000000000000000000000000000"),
                xr(
                    "000000000000000effffffffffffffff00000000000000000000000000000000",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("00000000000000000000000000000000000000000000000f0000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "00000000000000000000000000000000000000000000000effffffffffffffff",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("000000000f00000f00000000f00010000200030000f123000000000000000001"),
                x("000000000000002000f002000000002000000003000000000000000000007531"),
                xr(
                    "000000000effffeeff0ffe00f0000fe0020002fd00f122ffffffffffffff8ad0",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000009"),
                xr(
                    "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7",
                    0,
                ),
            ),
            // a= b= r=
            (
                x("0000000000000000000000000022872aa015d1317152fc14d55fc80000000000"),
                x("00000000000000000000000000000008d0f2fbba9bcb064d1e30038000000000"),
                xr(
                    "00000000000000000000000000228721cf22d576d587f5c7b72fc48000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SUB, None, &cases, None);
    }

    #[test]
    fn add() {
        let cases = [
            // a=0 b=0 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=1 r=2
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // a=-1 b=1 r=0
            (
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=-1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-7382179374129 b=12312412312412 r=4930232938283
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0x00000000000000000000000000000000000000000000000000000b32b4f6535c"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000047be8c86f2b",
                    0,
                ),
            ),
            // a=12312412312412 b=-7382179374129 r=4930232938283
            (
                x("0x00000000000000000000000000000000000000000000000000000b32b4f6535c"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000047be8c86f2b",
                    0,
                ),
            ),
            // a=-7382179374129 b=2412312412 r=-7379767061717
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0x000000000000000000000000000000000000000000000000000000008fc8f75c"),
                xr(
                    "0xfffffffffffffffffffffffffffffffffffffffffffffffffffff949c39b132b",
                    0,
                ),
            ),
            // a=2412312412 b=-7382179374129 r=-7379767061717
            (
                x("0x000000000000000000000000000000000000000000000000000000008fc8f75c"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                xr(
                    "0xfffffffffffffffffffffffffffffffffffffffffffffffffffff949c39b132b",
                    0,
                ),
            ),
        ];

        test_binary_op(ADD, None, &cases, None);
    }

    #[test]
    fn mul() {
        let cases = [
            // // a=0 b=0 r=0
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     xr(
            //         "0x0000000000000000000000000000000000000000000000000000000000000000",
            //         0,
            //     ),
            // ),
            // // a= b= r=
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     xr(
            //         "0x0000000000000000000000000000000000000000000000000000000000000000",
            //         0,
            //     ),
            // ),
            // // a= b= r=
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     xr(
            //         "0x0000000000000000000000000000000000000000000000000000000000000000",
            //         0,
            //     ),
            // ),
            // // a=1 b=1 r=1
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     xr(
            //         "0x0000000000000000000000000000000000000000000000000000000000000001",
            //         0,
            //     ),
            // ),
            // a=9 b=9 r=81
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000048",
                    0,
                ),
            ),
        ];

        test_binary_op(MUL, None, &cases, None);
    }

    #[test]
    fn gt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(GT, None, &cases, None);
    }

    #[test]
    fn lt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
        ];

        test_binary_op(LT, None, &cases, None);
    }

    #[test]
    fn mstore() {
        let max_result_size = 8 + 32; // multiple of 8
        let max_result_size = (max_result_size + 7) / 8 * 8;
        // expected results must be multiple of 8 bytes (i64/u64 value) or memory swap will spoil it
        let cases = [
            // offset=0 value= r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                xr(
                    "000000000f00000100000000f0000000100000000f00000000000000000f0000",
                    0,
                ),
            ),
            // offset=8 value= r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                xr("0000000000000000000000000f00000100000000f0000000100000000f00000000000000000f0000",8),
            ),
            // offset=3 value= r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                xr("000000000000000f00000100000000f0000000100000000f00000000000000000f0000",3),
            ),
        ];

        test_binary_op(MSTORE, None, &cases, Some(max_result_size));
    }

    #[test]
    fn mstore8() {
        let max_result_size = 8 + 32; // multiple of 8
        let max_result_size = (max_result_size + 7) / 8 * 8;
        // expected results must be multiple of 8 bytes (i64/u64 value) or memory swap will spoil it
        let cases = [
            // offset=0 value= r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0032"),
                x("3200000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f00af"),
                x("0000000000000000af0000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary_op(MSTORE8, None, &cases, Some(max_result_size));
    }

    #[test]
    fn keccak256() {
        // let max_result_size = 32; // multiple of 8
        // let max_result_size = (max_result_size + 7) / 8 * 8;
        // [(initial_bytecode, (a,b,result)), ...]
        let cases = [(
            compile_binary_op(
                MSTORE,
                &x("0000000000000000000000000000000000000000000000000000000000000000"),
                &xr(
                    "FFFFFFFF00000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                x("29045a592007d0c246ef02c2223570da9522d0cf0f73282c79a1bc8f0bb2c238"),
            ),
        )];
        for case in &cases {
            test_binary_op(KECCAK256, Some(&case.0), &[case.1.clone()], None);
        }
    }
}
