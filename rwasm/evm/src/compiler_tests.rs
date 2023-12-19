#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        consts::{SP_VAL_MEM_OFFSET_DEFAULT, VIRTUAL_STACK_TOP_DEFAULT},
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{
                ADD,
                ADDMOD,
                AND,
                BYTE,
                DIV,
                EQ,
                EXP,
                GT,
                ISZERO,
                KECCAK256,
                LT,
                MOD,
                MSTORE,
                MSTORE8,
                MUL,
                MULMOD,
                NOT,
                OR,
                POP,
                PUSH32,
                SAR,
                SDIV,
                SGT,
                SHL,
                SHR,
                SIGNEXTEND,
                SLT,
                SMOD,
                SUB,
                XOR,
            },
        },
    };
    use alloy_primitives::{hex, Bytes, U256};
    use fluentbase_runtime::{ExecutionResult, Runtime};
    use fluentbase_rwasm::rwasm::{
        BinaryFormat,
        BinaryFormatWriter,
        InstructionSet,
        ReducedModule,
    };
    use log::debug;

    fn x(hex: &str) -> Vec<u8> {
        let mut h = hex.replace(" ", "");
        h = h.replace("\n", "");
        if !h.starts_with("0x") {
            let mut t = "0x".to_string();
            t.push_str(&h);
            h = t;
        }
        let res = hex::decode(h.clone());
        if let Err(v) = res {
            panic!("failed to decode hex value '{:?}'", hex);
        }
        res.unwrap()
    }

    fn xr(hex: &str, le_from_offset: usize) -> Vec<u8> {
        let mut r = x(hex);
        // const SIZE: usize = EVM_WORD_BYTES;
        // r[le_from_offset..].chunks_mut(SIZE).for_each(|chunk| {
        //     if chunk.len() != EVM_WORD_BYTES {
        //         return;
        //     };
        //     for i in 0..SIZE / 2 {
        //         let tmp = chunk[i];
        //         chunk[i] = chunk[SIZE - i - 1];
        //         chunk[SIZE - i - 1] = tmp;
        //     }
        // });
        r
    }

    fn compile_binary_op(opcode: u8, a: &[u8], b: &[u8]) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(a);
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(b);
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    fn compile_ternary_op(opcode: u8, a: &[u8], b: &[u8], c: &[u8]) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(a);
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(b);
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(c);
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    fn compile_unary_op(opcode: u8, a: &[u8]) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        evm_bytecode.push(PUSH32);
        evm_bytecode.extend(a);
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    // /// @cases - &(a,b,result)
    // fn test_binary_op(
    //     opcode: u8,
    //     bytecode_preamble: Option<&[u8]>,
    //     cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)],
    //     force_memory_result_size_to: Option<usize>,
    // ) {
    //     for case in cases {
    //         let a = &case.0;
    //         let b = &case.1;
    //         let mut res_expected = case.2.clone();
    //         let mut evm_bytecode: Vec<u8> = vec![];
    //         bytecode_preamble.map(|v| evm_bytecode.extend(v));
    //         // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
    //         evm_bytecode.extend(compile_binary_op(opcode, a, b));
    //
    //         let mut global_memory = run_test(&evm_bytecode, force_memory_result_size_to);
    //         let res = &global_memory[0..res_expected.len()];
    //         if res_expected != res {
    //             debug!("a=            {:?}", a);
    //             debug!("b=            {:?}", b);
    //             debug!("res_expected= {:?}", res_expected);
    //             debug!("res=          {:?}", global_memory);
    //         }
    //         assert_eq!(res_expected, res);
    //     }
    // }

    /// @cases - &(a,b,result)
    fn test_unary_op(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[(Vec<u8>, Vec<u8>)],
        force_memory_result_size_to: Option<usize>,
        sp_is_zero: bool,
    ) {
        for case in cases {
            let a = &case.0;
            let res_expected = case.1.clone();

            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));
            evm_bytecode.extend(compile_unary_op(opcode, a));

            test_op(
                opcode,
                evm_bytecode,
                res_expected,
                force_memory_result_size_to,
                sp_is_zero,
            );
        }
    }

    /// @cases - &(a,b,result)
    fn test_binary_op(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)],
        force_memory_result_size_to: Option<usize>,
        sp_is_zero: bool,
    ) {
        for case in cases {
            let a = &case.0;
            let b = &case.1;
            let res_expected = case.2.clone();
            let res_expected_offset = SP_VAL_MEM_OFFSET_DEFAULT - res_expected.len();
            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));
            // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
            evm_bytecode.extend(compile_binary_op(opcode, a, b));

            test_op(
                opcode,
                evm_bytecode,
                res_expected,
                force_memory_result_size_to,
                sp_is_zero,
            );
        }
    }

    /// @cases - &(a,b,result)
    fn test_ternary_op(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)],
        force_memory_result_size_to: Option<usize>,
        sp_is_zero: bool,
    ) {
        for case in cases {
            let a = &case.0;
            let b = &case.1;
            let c = &case.2;
            let mut res_expected = case.3.clone();
            let res_expected_offset = SP_VAL_MEM_OFFSET_DEFAULT - res_expected.len();
            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));
            // TODO need evm preprocessing to automatically insert offset arg (PUSH0)
            evm_bytecode.extend(compile_ternary_op(opcode, a, b, c));

            test_op(
                opcode,
                evm_bytecode,
                res_expected,
                force_memory_result_size_to,
                sp_is_zero,
            );
        }
    }

    fn test_op(
        opcode: u8,
        evm_bytecode: Vec<u8>,
        res_expected: Vec<u8>,
        force_memory_result_size_to: Option<usize>,
        sp_is_zero: bool,
    ) {
        let res_expected_offset = SP_VAL_MEM_OFFSET_DEFAULT - res_expected.len();

        let mut global_memory = run_test(&evm_bytecode, force_memory_result_size_to);
        let sp_mem = &global_memory[SP_VAL_MEM_OFFSET_DEFAULT..SP_VAL_MEM_OFFSET_DEFAULT + 8];
        let sp_val = u64::from_le_bytes(sp_mem.try_into().unwrap());
        if sp_is_zero {
            assert_eq!(0, sp_val);
        } else {
            let res = &global_memory[res_expected_offset..res_expected_offset + res_expected.len()];
            assert_eq!(res_expected, res);
            assert_eq!(res_expected.len() as u64, sp_val);
        }
    }

    fn run_test(evm_bytecode_bytes: &Vec<u8>, force_memory_result_size: Option<usize>) -> Vec<u8> {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let import_linker = Runtime::<()>::new_linker();
        let mut compiler = EvmCompiler::new(&import_linker, false, evm_binary.as_ref());

        let mut preamble = InstructionSet::new();
        let virtual_stack_top = VIRTUAL_STACK_TOP_DEFAULT;
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

        let mut rmodule = ReducedModule::new(&rwasm_binary).unwrap();
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
        for (idx, log) in execution_result.tracer().logs.iter().enumerate() {
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
                "{}: opcode:{:x?} memory_changes:{:?}",
                idx, log.opcode, &memory_changes
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
            "global_memory (total len {}) {:?}",
            global_memory_len, &global_memory
        );
        let global_memory =
            global_memory[0..force_memory_result_size.unwrap_or(global_memory.len())].to_vec();
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

        test_binary_op(EQ, None, &cases, None, false);
    }

    #[test]
    fn iszero() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_unary_op(ISZERO, None, &cases, None, false);
    }

    #[test]
    fn not() {
        let cases = [
            // (
            //     x("0x000f00100300c000b0000a0000030000200001000600008000d0000200030010"),
            //     x("fff0ffeffcff3fff4ffff5fffffcffffdffffefff9ffff7fff2ffffdfffcffef"),
            // ),
            (
                x("0x 0000000000000001 0000000000000002 0000000000000003 0000000000000004"),
                x("fffffffffffffffe fffffffffffffffd fffffffffffffffc fffffffffffffffb"),
            ),
        ];

        test_unary_op(NOT, None, &cases, None, false);
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

        test_binary_op(SHL, None, &cases, None, false);
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

        test_binary_op(SHR, None, &cases, None, false);
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
            res[31] = i + 1;
            cases.push((
                idx,
                x("0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                res,
            ));
        }

        test_binary_op(BYTE, None, &cases, None, false);
    }

    // #[test]
    // fn and() {
    //     let cases = [
    //         (
    //             x("0x0000000000000000000000000000000000000000000000000000000000000000"),
    //             x("0x0000000000000000000000000000000000000000000000000000000000000000"),
    //             xr(
    //                 "0x0000000000000000000000000000000000000000000000000000000000000000",
    //                 0,
    //             ),
    //         ),
    //         (
    //             x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
    //             x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
    //             xr(
    //                 "0x0000010000000000000000010000a00080000000000000000000000020000001",
    //                 0,
    //             ),
    //         ),
    //     ];
    //
    //     test_binary_op(AND, None, &cases, None);
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

        test_binary_op(LT, None, &cases, None, false);
    }

    #[test]
    fn slt() {
        let cases = [
            (
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
        ];

        test_binary_op(SLT, None, &cases, None, false);
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

        test_binary_op(GT, None, &cases, None, false);
    }

    #[test]
    fn sgt() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SGT, None, &cases, None, false);
    }

    #[test]
    fn sar() {
        let cases = [
            (
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                x("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF0"),
                xr(
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
        ];

        test_binary_op(SAR, None, &cases, None, false);
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

        test_binary_op(ADD, None, &cases, None, false);
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
            (
                x("0000000000000000000000000022872aa015d1317152fc14d55fc80000000000"),
                x("00000000000000000000000000000008d0f2fbba9bcb064d1e30038000000000"),
                xr(
                    "00000000000000000000000000228721cf22d576d587f5c7b72fc48000000000",
                    0,
                ),
            ),
            (
                x("0x8000000000000000000000000000000000000000000000000000000000000000"),
                x("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
        ];

        test_binary_op(SUB, None, &cases, None, false);
    }

    #[test]
    fn addmod() {
        let cases = [
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000004",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
        ];

        test_ternary_op(ADDMOD, None, &cases, None, false);
    }

    #[test]
    fn signextend() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x00000000000000000000000000000000000000000000000000000000ff10aaaF"),
                xr(
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffaaaf",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                xr(
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x000000000000000000000000000000000000000000000000000000000000007f"),
                xr(
                    "0x000000000000000000000000000000000000000000000000000000000000007f",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000d"),
                x("0x0000000000000000000000000000000000007F00000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000007F00000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000d"),
                x("0x0000000000000000000000000000000000008F00000000000000000000000000"),
                xr(
                    "ffffffffffffffffffffffffffffffffffff8f00000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SIGNEXTEND, None, &cases, None, false);
    }

    #[test]
    fn mul() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
                x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
                xr(
                    "0x23397fc15242b5df02875774d26f2dd44d8b8eae3c41e8e88f44440caa00d0e4",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000020",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
        ];

        test_binary_op(MUL, None, &cases, None, false);
    }

    // TODO debug
    #[test]
    fn mulmod() {
        let cases = [
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000004",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000000000000000000005442fabcde12321e8984756"),
                x("0x00000000000000000000000000000000000000000f123f124f5f46f788ffafcf"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // TODO fix bug
            // (
            //     x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
            //     x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
            //     x("0x0000000000000000000000000000000000000000000000000000000008ffafcf"),
            //     xr(
            //         "0x0000000000000000000000000000000000000000000000000000000006561088",
            //         0,
            //     ),
            // ),
            (
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x000000000000000000000000000000000000000000000000000000000000000c"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000009",
                    0,
                ),
            ),
        ];

        test_ternary_op(MULMOD, None, &cases, None, false);
    }

    // TODO debug
    #[test]
    fn exp() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000025b7e5177122507d9"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000025b7e5177122507d9"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000064",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000004",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000008",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000010",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000051",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000d3"),
                xr(
                    "0x0000000000080000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000fc"),
                xr(
                    "0x1000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                xr(
                    "0x8000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x000000000000000000000000000000000000000000000000000000000000009c"),
                xr(
                    "0x0098a832626176ccbe4694b88a96dc004171b143838aa52d36988ddf2159d931",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x00000000000000000000000000000000000000000000000000000000000000a2"),
                xr(
                    "0xb2b6f77a278b4d09d6fd8182a7987cba5cc1c94195d05dc0786c0065f8db7c89",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                xr(
                    //
                    "0x8288753cb9b2e100000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x00000000000000000000000000000000000000000000000000000000000003e8"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(EXP, None, &cases, None, false);
    }

    #[test]
    fn div() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0xefffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                xr(
                    "0x00000000000000000000000000000000000000000000000000000001116c3527",
                    0,
                ),
            ),
            (
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                xr(
                    "0x000000000000000000000000000000000000000000000000000000001116c352",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000010",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    0,
                ),
            ),
        ];

        test_binary_op(DIV, None, &cases, None, false);
    }

    #[test]
    fn sdiv() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000021",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                xr(
                    "0x000000000000000000000000000000000000000000008c790a73e76a20fb8aa4",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000014d70ce7022e2de7e26734672778054107d2530"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                xr(
                    "0x000000000000000000000000000000000000000000008c790a00e76a00fb8aa4",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000014d70ce6dfd93fd2450565b5f141b9c107d2530"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                xr(
                    "0x000000000000000000000000000000000000000000008c790a00000000fb8aa4",
                    0,
                ),
            ),
            // a=   -1 -1 1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x000000000000000000247b41d58f447a82798e946efd4219e2b32d629d1d422b",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffec61d5769414ac99f25b30d9c6b44bfe9cb679"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffe199d0539cb6"),
                xr(
                    "0x00000000000000000000000000000000000000a5351659c6d8046540172a84a2",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffec61d5769414ac99f25b30d9c6b44bfe9cb679"),
                x("0x000000000000000000000000000000000000000000000000000000001a31ebea"),
                xr(
                    "0xffffffffffffffffffffffffffffffffff404718f404baac20abf5a87e655739",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000001e15f"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(SDIV, None, &cases, None, false);
    }

    // }

    #[test]
    fn and() {
        let cases = [
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
            (
                x(
                    "0x00 00 31 00 08 00 00 30   0f 00 00 07 00 00 a0 00   c0 00 00 00 00 00 30 00
            10 00 20 00 30 00 00 01",
                ),
                x("0x00 00 03 00 04 00 04 00   00 00 0a
            01 00 00 b0 00   a0 00 00 00 f0 00 00 00   07 00 00 04 20 0a 00 01"),
                x(
                    "0x00 00 01 00 00 00 00 00   00 00 00 01 00 00 a0 00   80 00 00 00 00 00 00
            00   00 00 00 00 20 00 00 01",
                ),
            ),
        ];

        test_binary_op(AND, None, &cases, None, false);
    }

    #[test]
    fn mod_impl() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                xr(
                    "0x00000000000000000000000000000000000000000000002163c2aa849ea53e83",
                    0,
                ),
            ),
            // -1 -1 0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // -1 1 1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
        ];

        test_binary_op(MOD, None, &cases, None, false);
    }

    #[test]
    fn smod_impl() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    0,
                ),
            ),
            (
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                xr(
                    "0x00000000000000000000000000000000000000000000002163c2aa849ea53e83",
                    0,
                ),
            ),
            // -1 -1 0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // -1 1 0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000000",
                    0,
                ),
            ),
            // -8 -3 -2
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                xr(
                    "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe",
                    0,
                ),
            ),
            // 11 -3 2
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000b"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                xr(
                    "0x0000000000000000000000000000000000000000000000000000000000000002",
                    0,
                ),
            ),
            // -11 3 -2
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff5"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                xr(
                    "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe",
                    0,
                ),
            ),
        ];

        test_binary_op(SMOD, None, &cases, None, false);
    }

    #[test]
    fn or() {
        let cases = [(
            x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
            x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
            x("0x000033000c0004300f000a070000b000e0000000f000300017002004300a0001"),
        )];

        test_binary_op(OR, None, &cases, None, false);
    }

    #[test]
    fn xor() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
                x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
                x("0x000032000c0004300f000a060000100060000000f000300017002004100a0000"),
            ),
        ];

        test_binary_op(XOR, None, &cases, None, false);
    }

    // TODO debug
    #[test]
    fn mstore() {
        let max_result_size = 40;
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

        test_binary_op(MSTORE, None, &cases, Some(max_result_size), false);
    }

    #[test]
    fn mstore8() {
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

        test_binary_op(MSTORE8, None, &cases, None, false);
    }

    #[test]
    fn keccak256() {
        // let max_result_size = 32; // multiple of 8
        // let max_result_size = (max_result_size + 7) / 8 * 8;
        // [(initial_bytecode, (a,b,result)), ...]
        let cases: &[(Vec<u8>, (Vec<u8>, Vec<u8>, Vec<u8>))] = &[(
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
        for case in cases {
            test_binary_op(KECCAK256, Some(&case.0), &[case.1.clone()], None, false);
        }
    }

    #[test]
    fn pop() {
        let cases = [(
            x("123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"),
            x("0000000000000000000000000000000000000000000000000000000000000000"),
        )];

        test_unary_op(POP, None, &cases, None, true);
    }
}
