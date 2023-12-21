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
                ADDRESS,
                AND,
                BYTE,
                CALLER,
                CALLVALUE,
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
    use alloy_primitives::{hex, Bytes};
    use fluentbase_codec::Encoder;
    use fluentbase_runtime::{ExecutionResult, Runtime, RuntimeContext};
    use fluentbase_rwasm::rwasm::{
        BinaryFormat,
        BinaryFormatWriter,
        InstructionSet,
        ReducedModule,
    };
    use fluentbase_sdk::evm::{Address, ContractInput};
    use log::debug;

    #[derive(Clone)]
    enum Case {
        NoArgs(Vec<u8>),
        Unary((Vec<u8>, Vec<u8>)),
        Binary((Vec<u8>, Vec<u8>, Vec<u8>)),
        Ternary((Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)),
    }

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

    fn compile_op_bytecode(opcode: u8, case: &Case) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        match case {
            Case::NoArgs(args) => {}
            Case::Unary(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
            }
            Case::Binary(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.1.clone());
            }
            Case::Ternary(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.1.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.2.clone());
            }
        }
        evm_bytecode.push(opcode);
        evm_bytecode
    }

    fn test_op_cases(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[Case],
        sp_is_zero: bool,
        res_is_in_memory_offset: Option<usize>,
    ) {
        for case in cases {
            let res_expected = match case {
                Case::NoArgs(v) => v.clone(),
                Case::Unary(v) => v.1.clone(),
                Case::Binary(v) => v.2.clone(),
                Case::Ternary(v) => v.3.clone(),
            };

            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));

            evm_bytecode.extend(compile_op_bytecode(opcode, case));

            test_op(
                opcode,
                evm_bytecode,
                res_expected,
                sp_is_zero,
                res_is_in_memory_offset,
            );
        }
    }

    fn test_op(
        opcode: u8,
        evm_bytecode: Vec<u8>,
        res_expected: Vec<u8>,
        sp_is_zero: bool,
        res_is_in_memory_offset: Option<usize>,
    ) {
        let mut global_memory = run_test(&evm_bytecode);
        let sp_mem = &global_memory[SP_VAL_MEM_OFFSET_DEFAULT..SP_VAL_MEM_OFFSET_DEFAULT + 8];
        let sp_val = u64::from_le_bytes(sp_mem.try_into().unwrap());
        if sp_is_zero {
            assert_eq!(0, sp_val);
        }
        if let Some(res_is_in_memory_offset) = res_is_in_memory_offset {
            let res_expected_offset = res_is_in_memory_offset;
            let res = &global_memory[res_expected_offset..res_expected_offset + res_expected.len()];
            assert_eq!(res_expected, res);
        } else {
            let res_expected_offset = SP_VAL_MEM_OFFSET_DEFAULT - res_expected.len();
            let res = &global_memory[res_expected_offset..res_expected_offset + res_expected.len()];
            assert_eq!(res_expected, res);
            assert_eq!(res_expected.len() as u64, sp_val);
        }
    }

    fn run_test(evm_bytecode_bytes: &Vec<u8>) -> Vec<u8> {
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
        let mut ctx = RuntimeContext::new(rwasm_binary);

        // TODO make customizable
        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = Address::new([1u8; 20]);
        let ci = contract_input.encode_to_vec(0);
        ctx = ctx.with_input(ci);

        let runtime = Runtime::new(ctx, &import_linker);
        let result = runtime.unwrap().call();
        // let result = Runtime::run(&rwasm_binary, &Vec::new(), 0);
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
        // let global_memory =
        //     global_memory[0..force_memory_result_size.unwrap_or(global_memory.len())].to_vec();
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
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(EQ, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn iszero() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(ISZERO, None, &cases.map(|v| Case::Unary(v)), false, None);
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

        test_op_cases(NOT, None, &cases.map(|v| Case::Unary(v)), false, None);
    }

    #[test]
    fn shl() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=4
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // shift=2 value=1 r=4
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                x("0xF000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFE"),
                x("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0x8000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xF000000000000000000000000000000000000000000000000000000000000000"),
                x("0xe000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(SHL, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn shr() {
        // [(shift, value, r), ...]
        let cases = [
            // shift=1 value=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // shift=2 value=4 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // shift=1 value=4 r=2
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x00000000000000000000000000000000000000000000000000000000000000FF"),
                x("0x000000000000000000000000000000000000000000000000000000000000000F"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            // externally checked
            (
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000101"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0x0000000000000000000000000F00000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // externally checked
            (
                x("0xF000000000000000000000000000000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(SHR, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn byte() {
        // [(idx, value, r), ...]
        let mut cases = vec![
            // shift=32 value=0xff..ff r=0
            Case::Binary((
                x("0x0000000000000000000000000000000000000000000000000000000000000020"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            // shift=33 value=0xff..ff r=0
            Case::Binary((
                x("0x0000000000000000000000000000000000000000000000000000000000000021"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
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
            cases.push(Case::Binary((
                idx,
                x("0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                res,
            )));
        }

        test_op_cases(BYTE, None, &cases, false, None);
    }
    #[test]
    fn lt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_op_cases(LT, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn slt() {
        let cases = [
            (
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_op_cases(SLT, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn gt() {
        let cases = [
            // a=-4 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-4 b=-5 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-5 b=-4 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30000 r=0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-30000 b=-30001 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=-30001 b=-30000 r=1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=2 b=1 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=1 b=2 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30000 b=30000 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=30001 b=30000 r=1
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=30000 b=30001 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000007530"),
                x("0x0000000000000000000000000000000000000000000000000000000000007531"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(GT, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn sgt() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(SGT, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn sar() {
        let cases = [
            (
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                x("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF0"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
        ];

        test_op_cases(SAR, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn add() {
        let cases = [
            // a=0 b=0 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=1 r=2
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // a=-1 b=1 r=0
            (
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=1 b=-1 r=0
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=-7382179374129 b=12312412312412 r=4930232938283
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0x00000000000000000000000000000000000000000000000000000b32b4f6535c"),
                x("0x0000000000000000000000000000000000000000000000000000047be8c86f2b"),
            ),
            // a=12312412312412 b=-7382179374129 r=4930232938283
            (
                x("0x00000000000000000000000000000000000000000000000000000b32b4f6535c"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0x0000000000000000000000000000000000000000000000000000047be8c86f2b"),
            ),
            // a=-7382179374129 b=2412312412 r=-7379767061717
            (
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0x000000000000000000000000000000000000000000000000000000008fc8f75c"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff949c39b132b"),
            ),
            // a=2412312412 b=-7382179374129 r=-7379767061717
            (
                x("0x000000000000000000000000000000000000000000000000000000008fc8f75c"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff94933d21bcf"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffff949c39b132b"),
            ),
        ];

        test_op_cases(ADD, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn sub() {
        let cases = [
            // cases where: a>=0, b>=0, a-b >=0
            // a=1 b=1 r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=2 b=1 r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // a=3 b=1 r=0
            (
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // a=30001 b=1 r=30000
            (
                x("0000000000000000000000000000000000000000000000000000000000007531"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000007530"),
            ),
            // a=b b=a r=0
            (
                x("0000000012000000012340000000f0000020000000f123000000030000000001"),
                x("0000000012000000012340000000f0000020000000f123000000030000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // a=291516405354026068686667777 b=30001 r=291516405354026068686637776
            (
                x("000000000000000000000000000000000000000000f123000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000007531"),
                x("000000000000000000000000000000000000000000f122ffffffffffffff8ad0"),
            ),
            // a= b= r=
            (
                x("000000010000001000111111f00010000200030000f123000000000000000001"),
                x("0000000000000010001000000000002000000003000000000000000000007531"),
                x("000000010000000000011111f0000fe0020002fd00f122ffffffffffffff8ad0"),
            ),
            // a= b= r=
            (
                x("000000000000000f00f001000000000000000000000000000000000000000000"),
                x("000000000000000000f002000000000000000000000000000000000000000000"),
                x("000000000000000effffff000000000000000000000000000000000000000000"),
            ),
            // a= b= r=
            (
                x("000000000000000f000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000100000000000000000000000000000000"),
                x("000000000000000effffffffffffffff00000000000000000000000000000000"),
            ),
            // a= b= r=
            (
                x("00000000000000000000000000000000000000000000000f0000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("00000000000000000000000000000000000000000000000effffffffffffffff"),
            ),
            // a= b= r=
            (
                x("000000000f00000f00000000f00010000200030000f123000000000000000001"),
                x("000000000000002000f002000000002000000003000000000000000000007531"),
                x("000000000effffeeff0ffe00f0000fe0020002fd00f122ffffffffffffff8ad0"),
            ),
            // a= b= r=
            (
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000009"),
                x("fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7"),
            ),
            (
                x("0000000000000000000000000022872aa015d1317152fc14d55fc80000000000"),
                x("00000000000000000000000000000008d0f2fbba9bcb064d1e30038000000000"),
                x("00000000000000000000000000228721cf22d576d587f5c7b72fc48000000000"),
            ),
            (
                x("0x8000000000000000000000000000000000000000000000000000000000000000"),
                x("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_op_cases(SUB, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn addmod() {
        let cases = [
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_op_cases(ADDMOD, None, &cases.map(|v| Case::Ternary(v)), false, None);
    }

    #[test]
    fn signextend() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x00000000000000000000000000000000000000000000000000000000ff10aaaF"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffaaaf"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x000000000000000000000000000000000000000000000000000000000000007f"),
                x("0x000000000000000000000000000000000000000000000000000000000000007f"),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000d"),
                x("0x0000000000000000000000000000000000007F00000000000000000000000000"),
                x("0x0000000000000000000000000000000000007F00000000000000000000000000"),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000d"),
                x("0x0000000000000000000000000000000000008F00000000000000000000000000"),
                x("ffffffffffffffffffffffffffffffffffff8f00000000000000000000000000"),
            ),
        ];

        test_op_cases(
            SIGNEXTEND,
            None,
            &cases.map(|v| Case::Binary(v)),
            false,
            None,
        );
    }

    #[test]
    fn mul() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
                x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
                x("0x23397fc15242b5df02875774d26f2dd44d8b8eae3c41e8e88f44440caa00d0e4"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x0000000000000000000000000000000000000000000000000000000000000020"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_op_cases(MUL, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn mulmod() {
        let cases = [
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            (
                x("0x000000000000000000000000000000000000000005442fabcde12321e8984756"),
                x("0x00000000000000000000000000000000000000000f123f124f5f46f788ffafcf"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            // TODO fix bug
            // (
            //     x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
            //     x("0x0423f423fadfec123cd33248461674153717263575442fabcde12321e8984756"),
            //     x("0x0000000000000000000000000000000000000000000000000000000008ffafcf"),
            //     x("0x0000000000000000000000000000000000000000000000000000000006561088"),
            // ),
            (
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x000000000000000000000000000000000000000000000000000000000000000c"),
                x("0x0000000000000000000000000000000000000000000000000000000000000009"),
            ),
        ];

        test_op_cases(MULMOD, None, &cases.map(|v| Case::Ternary(v)), false, None);
    }

    #[test]
    fn exp() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000025b7e5177122507d9"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000025b7e5177122507d9"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000000000a"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000008"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000004"),
                x("0x0000000000000000000000000000000000000000000000000000000000000051"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000d3"),
                x("0x0000000000080000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000fc"),
                x("0x1000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                x("0x8000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000100"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x000000000000000000000000000000000000000000000000000000000000009c"),
                x("0x0098a832626176ccbe4694b88a96dc004171b143838aa52d36988ddf2159d931"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x00000000000000000000000000000000000000000000000000000000000000a2"),
                x("0xb2b6f77a278b4d09d6fd8182a7987cba5cc1c94195d05dc0786c0065f8db7c89"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x(
                    //
                    "0x8288753cb9b2e100000000000000000000000000000000000000000000000000",
                ),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x00000000000000000000000000000000000000000000000000000000000003e8"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(EXP, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn div() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0xefffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0x00000000000000000000000000000000000000000000000000000001116c3527"),
            ),
            (
                x("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x00000000efaffffbffffffffffffffff1fff1fff2ffff5ffff8ffff2fff3ffff"),
                x("0x000000000000000000000000000000000000000000000000000000001116c352"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000010"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
        ];

        test_op_cases(DIV, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn sdiv() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000021"),
            ),
            (
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x000000000000000000000000000000000000000000008c790a73e76a20fb8aa4"),
            ),
            (
                x("0x000000000000000000000000014d70ce7022e2de7e26734672778054107d2530"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x000000000000000000000000000000000000000000008c790a00e76a00fb8aa4"),
            ),
            (
                x("0x000000000000000000000000014d70ce6dfd93fd2450565b5f141b9c107d2530"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x000000000000000000000000000000000000000000008c790a00000000fb8aa4"),
            ),
            // a=   -1 -1 1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x000000000000000000247b41d58f447a82798e946efd4219e2b32d629d1d422b"),
            ),
            (
                x("0xffffffffffffffffffffffffffec61d5769414ac99f25b30d9c6b44bfe9cb679"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffe199d0539cb6"),
                x("0x00000000000000000000000000000000000000a5351659c6d8046540172a84a2"),
            ),
            (
                x("0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0xffffffffffffffffffdb84be2a70bb857d86716b9102bde61d4cd29d62e2bdd5"),
            ),
            (
                x("0xffffffffffffffffffffffffffec61d5769414ac99f25b30d9c6b44bfe9cb679"),
                x("0x000000000000000000000000000000000000000000000000000000001a31ebea"),
                x("0xffffffffffffffffffffffffffffffffff404718f404baac20abf5a87e655739"),
            ),
            (
                x("0x000000000000000000000000000000000000000000000000000000000001e15f"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(SDIV, None, &cases.map(|v| Case::Binary(v)), false, None);
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

        test_op_cases(AND, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn mod_impl() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x00000000000000000000000000000000000000000000002163c2aa849ea53e83"),
            ),
            // -1 -1 0
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            // -1 1 1
            (
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_op_cases(MOD, None, &cases.map(|v| Case::Binary(v)), false, None);
    }

    #[test]
    fn smod_impl() {
        let cases = [
            Case::Binary((
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Binary((
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            )),
            Case::Binary((
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x00000000000000000000000000000000000000000000002163c2aa849ea53e83"),
            )),
            Case::Binary((
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Binary((
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Binary((
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
            )),
            Case::Binary((
                x("0x000000000000000000000000000000000000000000000000000000000000000b"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            )),
            Case::Binary((
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff5"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
            )),
        ];

        test_op_cases(SMOD, None, &cases, false, None);
    }

    #[test]
    fn or() {
        let cases = [Case::Binary((
            x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
            x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
            x("0x000033000c0004300f000a070000b000e0000000f000300017002004300a0001"),
        ))];

        test_op_cases(OR, None, &cases, false, None);
    }

    #[test]
    fn xor() {
        let cases = [
            Case::Binary((
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Binary((
                x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
                x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
                x("0x000032000c0004300f000a060000100060000000f000300017002004100a0000"),
            )),
        ];

        test_op_cases(XOR, None, &cases, false, None);
    }

    #[test]
    fn mstore() {
        let cases = [
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("0000000000000000000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("000000000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
        ];

        test_op_cases(MSTORE, None, &cases, true, Some(0));
    }

    #[test]
    fn mstore8() {
        let cases = [
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0032"),
                x("3200000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f00af"),
                x("0000000000000000af0000000000000000000000000000000000000000000000"),
            )),
        ];

        test_op_cases(MSTORE8, None, &cases, true, Some(0));
    }

    #[test]
    fn address() {
        let cases = [Case::NoArgs(x(
            "123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234",
        ))];

        test_op_cases(ADDRESS, None, &cases, true, None);
    }

    #[test]
    fn caller() {
        let cases = [Case::NoArgs(x(
            "123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234",
        ))];

        test_op_cases(CALLER, None, &cases, true, None);
    }

    // #[test]
    // fn callvalue() {
    //     let cases = [Case::NoArgs(x(""))];
    //
    //     test_op_cases(CALLVALUE, None, &cases, true, None);
    // }

    #[test]
    fn keccak256() {
        let cases = [(
            compile_binary_op(
                MSTORE,
                &x("0000000000000000000000000000000000000000000000000000000000000000"),
                &x("FFFFFFFF00000000000000000000000000000000000000000000000000000000"),
            ),
            Case::Binary((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                x("29045a592007d0c246ef02c2223570da9522d0cf0f73282c79a1bc8f0bb2c238"),
            )),
        )];
        for case in cases {
            test_op_cases(KECCAK256, Some(&case.0), &[case.1.clone()], false, None);
        }
    }

    #[test]
    fn pop() {
        let cases = [Case::Unary((
            x("123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"),
            x(""),
        ))];

        test_op_cases(POP, None, &cases, true, None);
    }

    #[test]
    fn compound_or() {
        let mut preamble = vec![];
        // 1|0=1
        preamble.extend(compile_binary_op(
            OR,
            &x("0000000000000000000000000000000000000000000000000000000000000001"),
            &x("0000000000000000000000000000000000000000000000000000000000000000"),
        ));
        // 1|2=3
        let cases = &[Case::Unary((
            x("0000000000000000000000000000000000000000000000000000000000000002"),
            x("0000000000000000000000000000000000000000000000000000000000000003"),
        ))];

        test_op_cases(OR, Some(&preamble), cases, false, None);
    }

    #[test]
    fn compound_add() {
        let mut preamble = vec![];
        preamble.extend(compile_binary_op(
            ADD,
            &x("0000000000000000000000000000000000000000000000000000000000000001"),
            &x("0000000000000000000000000000000000000000000000000000000000000002"),
        ));
        preamble.extend(compile_binary_op(
            ADD,
            &x("0000000000000000000000000000000000000000000000000000000000000003"),
            &x("0000000000000000000000000000000000000000000000000000000000000004"),
        ));
        preamble.push(ADD);
        let cases = &[Case::Unary((
            x("0000000000000000000000000000000000000000000000000000000000000005"),
            x("000000000000000000000000000000000000000000000000000000000000000f"),
        ))];

        test_op_cases(ADD, Some(&preamble), cases, false, None);
    }

    #[test]
    fn compound_mul_add_div() {
        let mut preamble = vec![];
        // 2*3=6
        preamble.extend(compile_binary_op(
            MUL,
            &x("0000000000000000000000000000000000000000000000000000000000000002"),
            &x("0000000000000000000000000000000000000000000000000000000000000003"),
        ));
        //6+7=12
        preamble.extend(compile_unary_op(
            ADD,
            &x("0000000000000000000000000000000000000000000000000000000000000007"),
        ));

        //12/5=2
        let cases = &[Case::Unary((
            x("0000000000000000000000000000000000000000000000000000000000000005"),
            x("0000000000000000000000000000000000000000000000000000000000000002"),
        ))];

        test_op_cases(DIV, Some(&preamble), cases, false, None);
    }
}
