#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        consts::{SP_BASE_MEM_OFFSET_DEFAULT, VIRTUAL_STACK_TOP_DEFAULT},
        translator::{
            instruction_result::InstructionResult,
            instructions::{
                opcode,
                opcode::{
                    ADD,
                    ADDMOD,
                    ADDRESS,
                    AND,
                    BASEFEE,
                    BLOBBASEFEE,
                    BLOBHASH,
                    BLOCKHASH,
                    BYTE,
                    CALLDATACOPY,
                    CALLDATALOAD,
                    CALLDATASIZE,
                    CALLER,
                    CALLVALUE,
                    CHAINID,
                    CODESIZE,
                    COINBASE,
                    DIFFICULTY,
                    DIV,
                    DUP1,
                    DUP2,
                    EQ,
                    EXP,
                    GAS,
                    GASLIMIT,
                    GASPRICE,
                    GT,
                    ISZERO,
                    JUMP,
                    JUMPI,
                    KECCAK256,
                    LT,
                    MCOPY,
                    MLOAD,
                    MOD,
                    MSIZE,
                    MSTORE,
                    MSTORE8,
                    MUL,
                    MULMOD,
                    NOT,
                    NUMBER,
                    OR,
                    ORIGIN,
                    POP,
                    PUSH32,
                    RETURN,
                    SAR,
                    SDIV,
                    SGT,
                    SHL,
                    SHR,
                    SIGNEXTEND,
                    SLOAD,
                    SLT,
                    SMOD,
                    SSTORE,
                    STOP,
                    SUB,
                    SWAP1,
                    SWAP2,
                    TIMESTAMP,
                    TSTORE,
                    XOR,
                },
            },
        },
        utilities::EVM_WORD_BYTES,
    };
    use alloy_primitives::{hex, Bytes, B256};
    use fluentbase_codec::Encoder;
    use fluentbase_runtime::{ExecutionResult, Runtime, RuntimeContext};
    use fluentbase_rwasm::{
        engine::bytecode::Instruction,
        rwasm::{BinaryFormat, BinaryFormatWriter, InstructionSet, ReducedModule},
    };
    use fluentbase_sdk::evm::{Address, ContractInput, U256};
    use log::debug;

    static CONTRACT_ADDRESS: [u8; 20] = [1; 20]; // Address - 20 bytes
    static CONTRACT_CALLER: [u8; 20] = [2; 20]; // Address - 20 bytes
    static CONTRACT_VALUE: [u8; 32] = [3; 32]; // U256 - 32 bytes
    static SYSTEM_CODESIZE: [u8; 4] = [4; 4]; // u32 - 4 bytes
    static CONTRACT_INPUT: &[u8] = &[
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32, 33, 34, 25, 26, 37, 38, 39, 40, 41,
    ];
    static HOST_CHAINID: [u8; 8] = [5; 8]; // u64 - 8 bytes
    static HOST_BASEFEE: [u8; 32] = [6; 32]; // U256 - 32 bytes
    static HOST_BLOCKHASH: [u8; 32] = [7; 32]; // B256 - 32 bytes
    static HOST_COINBASE: [u8; 20] = [8; 20]; // Address - 20 bytes
    static HOST_GASLIMIT: [u8; 8] = [9; 8]; // u64 - 8 bytes
    static HOST_NUMBER: [u8; 8] = [10; 8]; // u64 - 8 bytes
    static HOST_TIMESTAMP: [u8; 8] = [11; 8]; // u64 - 8 bytes
    static HOST_ENV_DIFFICULTY: [u8; 8] = [12; 8]; // u64 - 8 bytes
    static HOST_ENV_BLOBBASEFEE: [u8; 8] = [13; 8]; // u64 - 8 bytes
    static HOST_ENV_GASPRICE: [u8; 32] = [14; 32]; // u256 - 32 bytes
    static HOST_ENV_ORIGIN: [u8; 20] = [15; 20]; // Address - 20 bytes
    static HOST_ENV_BLOB_HASHES: &[[u8; 32]] = &[[1; 32], [2; 32], [3; 32]];

    #[derive(Clone)]
    enum Case {
        // result_expected
        Args0(Vec<u8>),
        // result_expected a
        Args1((Vec<u8>, Vec<u8>)),
        // result_expected a b
        Args2((Vec<u8>, Vec<u8>, Vec<u8>)),
        // result_expected a b c
        Args3((Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)),
    }

    #[derive(Clone)]
    enum ResultLocation {
        Stack,
        Memory(usize),
        Output(usize),
    }

    fn compile_op_with_args_bytecode(opcode: Option<u8>, case: &Case) -> Vec<u8> {
        let mut evm_bytecode: Vec<u8> = vec![];
        match case {
            Case::Args0(args) => {}
            Case::Args1(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
            }
            Case::Args2(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.1.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
            }
            Case::Args3(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.2.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.1.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
            }
        }
        if let Some(opcode) = opcode {
            evm_bytecode.push(opcode);
        }
        evm_bytecode
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

    fn test_op_cases(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[Case],
        sp: Option<i32>,
        result_location: ResultLocation,
    ) {
        for case in cases {
            let res_expected = match case {
                Case::Args0(v) => v.clone(),
                Case::Args1(v) => v.1.clone(),
                Case::Args2(v) => v.2.clone(),
                Case::Args3(v) => v.3.clone(),
            };

            let mut evm_bytecode: Vec<u8> = vec![];
            bytecode_preamble.map(|v| evm_bytecode.extend(v));

            evm_bytecode.extend(compile_op_with_args_bytecode(
                if opcode == STOP { None } else { Some(opcode) },
                case,
            ));

            test_op(
                opcode,
                evm_bytecode,
                res_expected,
                sp,
                result_location.clone(),
            );
        }
    }

    fn test_op(
        opcode: u8,
        evm_bytecode: Vec<u8>,
        res_expected: Vec<u8>,
        sp: Option<i32>,
        result_location: ResultLocation,
    ) {
        let (mut global_memory, output) = run_test(&evm_bytecode);
        let sp_mem = &global_memory[SP_BASE_MEM_OFFSET_DEFAULT..SP_BASE_MEM_OFFSET_DEFAULT + 8];
        let sp_val = u64::from_le_bytes(sp_mem.try_into().unwrap()) as usize;
        if let Some(sp) = sp {
            if sp < 0 {
                assert!(sp_val > 0);
            } else {
                assert_eq!(sp as usize, sp_val);
            }
        }
        match result_location {
            ResultLocation::Stack => {
                let res_expected_offset = SP_BASE_MEM_OFFSET_DEFAULT - sp_val;
                let res =
                    &global_memory[res_expected_offset..res_expected_offset + res_expected.len()];
                assert_eq!(res_expected, res);
                // assert_eq!(res_expected.len(), sp_val);
            }
            ResultLocation::Memory(offset) => {
                let res = &global_memory[offset..offset + res_expected.len()];
                assert_eq!(res_expected, res);
            }
            ResultLocation::Output(offset) => {
                let res = &output[offset..offset + res_expected.len()];
                assert_eq!(res_expected, res);
            }
        }
    }

    fn run_test(evm_bytecode_bytes: &Vec<u8>) -> (Vec<u8>, Vec<u8>) {
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
        let res = compiler.run(Some(&preamble), None);
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

        let mut global_memory = vec![];
        let mut global_memory_len: usize = 0;
        let mut runtime_ctx = RuntimeContext::new(rwasm_binary);

        // TODO make it customizable
        let mut contract_input = ContractInput::default();
        contract_input.contract_address = Address::new(CONTRACT_ADDRESS);
        contract_input.contract_caller = Address::new(CONTRACT_CALLER);
        contract_input.contract_value = U256::from_be_bytes(CONTRACT_VALUE);
        contract_input.contract_code_size = u32::from_be_bytes(SYSTEM_CODESIZE);
        contract_input.contract_input = CONTRACT_INPUT.to_vec();
        contract_input.env_chain_id = u64::from_be_bytes(HOST_CHAINID);
        contract_input.block_base_fee = U256::from_be_bytes(HOST_BASEFEE);
        contract_input.block_hash = B256::new(HOST_BLOCKHASH);
        contract_input.block_coinbase = Address::from(HOST_COINBASE);
        contract_input.block_gas_limit = u64::from_be_bytes(HOST_GASLIMIT);
        contract_input.block_number = u64::from_be_bytes(HOST_NUMBER);
        contract_input.block_timestamp = u64::from_be_bytes(HOST_TIMESTAMP);
        contract_input.block_difficulty = u64::from_be_bytes(HOST_ENV_DIFFICULTY);
        contract_input.tx_blob_gas_price = u64::from_be_bytes(HOST_ENV_BLOBBASEFEE);
        contract_input.tx_gas_price = U256::from_be_bytes(HOST_ENV_GASPRICE);
        contract_input.tx_caller = Address::new(HOST_ENV_ORIGIN);
        contract_input.tx_blob_hashes = HOST_ENV_BLOB_HASHES
            .iter()
            .map(|v| B256::from_slice(v))
            .collect();
        let ci = contract_input.encode_to_vec(0);
        runtime_ctx = runtime_ctx.with_input(ci);

        let runtime = Runtime::new(runtime_ctx, &import_linker);
        let mut runtime = runtime.unwrap();
        let result = runtime.call();
        // let result = Runtime::run(&rwasm_binary, &Vec::new(), 0);
        assert!(result.is_ok());
        let execution_result: ExecutionResult<()> = result.unwrap();
        for (idx, log) in execution_result.tracer().logs.iter().enumerate() {
            // if log.memory_changes.len() <= 0 {
            //     continue;
            // };
            let memory_changes = &log.memory_changes;
            let stack = &log.stack;
            let prev_opcode = if idx > 0 {
                Some(execution_result.tracer().logs[idx - 1].opcode)
            } else {
                None
            };
            debug!(
                "idx {}: opcode:{:?} (prev:{:?}) memory_changes:{:?} stack:{:?}",
                idx,
                log.opcode,
                prev_opcode.unwrap_or(Instruction::Unreachable),
                &memory_changes,
                stack
            );
            for change in memory_changes {
                let offset_start = change.offset as usize;
                let offset_end = offset_start + change.len as usize;
                if global_memory.len() <= offset_end {
                    global_memory.resize(offset_end + 1, 0);
                }
                global_memory[offset_start..offset_end].copy_from_slice(&change.data);
                if offset_end > global_memory_len {
                    global_memory_len = offset_end;
                }
            }
        }
        debug!(
            "\nruntime.store.data().output() {:?}\n",
            runtime.data().output()
        );
        assert_eq!(execution_result.data().exit_code(), 0);

        debug!(
            "\nglobal_memory ({}): {:?}\n",
            global_memory_len, &global_memory
        );

        (global_memory, runtime.data().output().clone())
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

        test_op_cases(
            EQ,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn iszero() {
        let cases = [
            (
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            // (
            //     x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
            // (
            //     x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
            // (
            //     x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
            //     x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            // ),
        ];

        test_op_cases(
            ISZERO,
            None,
            &cases.map(|v| Case::Args1(v)),
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn not() {
        let cases = [
            (
                x("0x000f00100300c000b0000a0000030000200001000600008000d0000200030010"),
                x("fff0ffeffcff3fff4ffff5fffffcffffdffffefff9ffff7fff2ffffdfffcffef"),
            ),
            (
                x("0x 0000000000000001 0000000000000002 0000000000000003 0000000000000004"),
                x("fffffffffffffffe fffffffffffffffd fffffffffffffffc fffffffffffffffb"),
            ),
        ];

        test_op_cases(
            NOT,
            None,
            &cases.map(|v| Case::Args1(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SHL,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SHR,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn byte() {
        // [(idx, value, r), ...]
        let mut cases = vec![
            // shift=32 value=0xff..ff r=0
            Case::Args2((
                x("0x0000000000000000000000000000000000000000000000000000000000000020"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            // shift=33 value=0xff..ff r=0
            Case::Args2((
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
            cases.push(Case::Args2((
                idx,
                x("0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                res,
            )));
        }

        test_op_cases(BYTE, None, &cases, Some(-1), ResultLocation::Stack);
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

        test_op_cases(
            LT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SLT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            GT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SGT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SAR,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            ADD,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SUB,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            ADDMOD,
            None,
            &cases.map(|v| Case::Args3(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
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

        test_op_cases(
            MUL,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            MULMOD,
            None,
            &cases.map(|v| Case::Args3(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            EXP,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            DIV,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            SDIV,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            AND,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
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

        test_op_cases(
            MOD,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn smod_impl() {
        let cases = [
            Case::Args2((
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Args2((
                x("0x0000000000000000000000000000000000000000000000000000000000000064"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
            )),
            Case::Args2((
                x("0x000000000000000000000000014d70cf811caff6fb45deb45abffe262f2263b3"),
                x("0x00000000000000000000000000000000000000000000025faaf6a5e9300e9a6c"),
                x("0x00000000000000000000000000000000000000000000002163c2aa849ea53e83"),
            )),
            Case::Args2((
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Args2((
                x("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                x("0x0000000000000000000000000000000000000000000000000000000000000001"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Args2((
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
            )),
            Case::Args2((
                x("0x000000000000000000000000000000000000000000000000000000000000000b"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd"),
                x("0x0000000000000000000000000000000000000000000000000000000000000002"),
            )),
            Case::Args2((
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff5"),
                x("0x0000000000000000000000000000000000000000000000000000000000000003"),
                x("0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe"),
            )),
        ];

        test_op_cases(SMOD, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn or() {
        let cases = [Case::Args2((
            x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
            x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
            x("0x000033000c0004300f000a070000b000e0000000f000300017002004300a0001"),
        ))];

        test_op_cases(OR, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn xor() {
        let cases = [
            Case::Args2((
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
                x("0x0000000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Args2((
                x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
                x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
                x("0x000032000c0004300f000a060000100060000000f000300017002004100a0000"),
            )),
        ];

        test_op_cases(XOR, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn mstore() {
        let cases = [
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("0000000000000000000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0000"),
                x("000000000000000f00000100000000f0000000100000000f00000000000000000f0000"),
            )),
        ];

        test_op_cases(MSTORE, None, &cases, Some(0), ResultLocation::Memory(0));
    }

    #[test]
    fn mstore8() {
        let cases = [
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f0032"),
                x("3200000000000000000000000000000000000000000000000000000000000000"),
            )),
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f00af"),
                x("0000000000000000af0000000000000000000000000000000000000000000000"),
            )),
        ];

        test_op_cases(MSTORE8, None, &cases, Some(0), ResultLocation::Memory(0));
    }

    #[test]
    fn mload() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(MSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("00000000000000000000000000000000000000000000000000000000000000FF"),
                vec![],
            )),
        ));
        let cases = [
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("00000000000000000000000000000000000000000000000000000000000000FF"),
            )),
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("000000000000000000000000000000000000000000000000000000000000FF00"),
            )),
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                x("0000000000000000000000000000000000000000000000000000000000FF0000"),
            )),
        ];

        test_op_cases(
            MLOAD,
            Some(&preamble),
            &cases,
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn msize() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(MSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("00000000000000000000000000000000000000000000000000000000000000FF"),
                vec![],
            )),
        ));
        let cases = [Case::Args0(x(
            "0000000000000000000000000000000000000000000000000000000000000014",
        ))];

        test_op_cases(
            MSIZE,
            Some(&preamble),
            &cases,
            Some(-1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn mcopy() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(MSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"),
                vec![],
            )),
        ));
        let cases = [
            Case::Args3((
                // dest src len
                x("0000000000000000000000000000000000000000000000000000000000000020"), // dst 32
                x("0000000000000000000000000000000000000000000000000000000000000000"), // src 0
                x("0000000000000000000000000000000000000000000000000000000000000005"), // size 5
                x("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f200102030405000000000000000000"),
            )),
            Case::Args3((
                // dest src len
                x("0000000000000000000000000000000000000000000000000000000000000020"), // dst 32
                x("0000000000000000000000000000000000000000000000000000000000000003"), // src 3
                x("0000000000000000000000000000000000000000000000000000000000000006"), // size 6
                x("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f200405060708090000000000000000"),
            )),
            Case::Args3((
                // dest src len
                x("0000000000000000000000000000000000000000000000000000000000000020"), // dst 32
                x("0000000000000000000000000000000000000000000000000000000000000003"), // src 3
                x("0000000000000000000000000000000000000000000000000000000000000000"), // size 0
                x("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f200000000000000000000000000000"),
            )),
        ];

        test_op_cases(
            MCOPY,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Memory(0),
        );
    }

    #[test]
    fn caller() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - CONTRACT_ADDRESS.len()];
            v.extend(&CONTRACT_CALLER);
            v
        })];

        test_op_cases(CALLER, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn address() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - CONTRACT_ADDRESS.len()];
            v.extend(&CONTRACT_ADDRESS);
            v
        })];

        test_op_cases(ADDRESS, None, &cases, Some(-1), ResultLocation::Stack);
    }

    // #[ignore]
    #[test]
    fn calldatasize() {
        let mut cases = vec![];
        let len_be = CONTRACT_INPUT.len().to_be_bytes();
        let mut v = [0u8; 32];
        v[32 - len_be.len()..].copy_from_slice(&len_be);
        cases.push(Case::Args0(v.to_vec()));

        test_op_cases(CALLDATASIZE, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn calldataload() {
        let mut cases = vec![];

        let start_idx = 1;
        for idx in start_idx..CONTRACT_INPUT.len() - 1 {
            let mut v = [0u8; EVM_WORD_BYTES];
            let idx_be = idx.to_be_bytes();
            v[EVM_WORD_BYTES - idx_be.len()..].copy_from_slice(&idx_be);

            let start_idx = idx;
            let end_idx = idx
                + if CONTRACT_INPUT.len() - idx >= EVM_WORD_BYTES {
                    EVM_WORD_BYTES
                } else {
                    CONTRACT_INPUT.len() - idx
                };
            let mut res_expected = CONTRACT_INPUT[start_idx..end_idx].to_vec();
            res_expected.resize(EVM_WORD_BYTES, 0);
            cases.push(Case::Args1((v.to_vec(), res_expected)));
        }

        test_op_cases(CALLDATALOAD, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn calldatacopy() {
        let mut cases = vec![];

        let mut dest_offset = [0u8; EVM_WORD_BYTES];
        let mut offset = [0u8; EVM_WORD_BYTES];
        let mut size = [0u8; EVM_WORD_BYTES];

        let mut res_expected = [0u8; EVM_WORD_BYTES];
        cases.push(Case::Args3((
            dest_offset.to_vec(),
            offset.to_vec(),
            size.to_vec(),
            res_expected.to_vec(),
        )));

        size[EVM_WORD_BYTES - 1] = EVM_WORD_BYTES as u8;
        let mut res_expected = vec![];
        let res = &CONTRACT_INPUT[offset[0] as usize..size[EVM_WORD_BYTES - 1] as usize];
        res_expected = res.to_vec();
        res_expected.resize(
            (res_expected.len() + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES,
            0,
        );
        cases.push(Case::Args3((
            dest_offset.to_vec(),
            offset.to_vec(),
            size.to_vec(),
            res_expected,
        )));

        size[EVM_WORD_BYTES - 1] = EVM_WORD_BYTES as u8 + 1;
        let mut res_expected = vec![];
        let res = &CONTRACT_INPUT[offset[0] as usize..size[EVM_WORD_BYTES - 1] as usize];
        res_expected = res.to_vec();
        res_expected.resize(
            (res_expected.len() + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES,
            0,
        );
        cases.push(Case::Args3((
            dest_offset.to_vec(),
            offset.to_vec(),
            size.to_vec(),
            res_expected,
        )));

        size[EVM_WORD_BYTES - 1] = 13;
        let mut res_expected = vec![];
        let res = &CONTRACT_INPUT[offset[0] as usize..size[EVM_WORD_BYTES - 1] as usize];
        res_expected = res.to_vec();
        res_expected.resize(
            (res_expected.len() + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES,
            0,
        );
        cases.push(Case::Args3((
            dest_offset.to_vec(),
            offset.to_vec(),
            size.to_vec(),
            res_expected,
        )));

        test_op_cases(
            CALLDATACOPY,
            None,
            &cases,
            Some(0),
            ResultLocation::Memory(0),
        );
    }

    #[test]
    fn callvalue() {
        let cases = [Case::Args0(CONTRACT_VALUE.to_vec())];

        test_op_cases(CALLVALUE, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn keccak256() {
        let cases = [(
            compile_op_with_args_bytecode(
                Some(MSTORE),
                &Case::Args2((
                    x("0000000000000000000000000000000000000000000000000000000000000000"),
                    x("FFFFFFFF00000000000000000000000000000000000000000000000000000000"),
                    vec![],
                )),
            ),
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                x("29045a592007d0c246ef02c2223570da9522d0cf0f73282c79a1bc8f0bb2c238"),
            )),
        )];
        for case in cases {
            test_op_cases(
                KECCAK256,
                Some(&case.0),
                &[case.1.clone()],
                Some(-1),
                ResultLocation::Stack,
            );
        }
    }

    #[test]
    fn codesize() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - SYSTEM_CODESIZE.len()];
            v.extend(&SYSTEM_CODESIZE);
            v
        })];

        test_op_cases(CODESIZE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn chainid() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_CHAINID.len()];
            v.extend(&HOST_CHAINID);
            v
        })];

        test_op_cases(CHAINID, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn basefee() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_BASEFEE.len()];
            v.extend(&HOST_BASEFEE);
            v
        })];

        test_op_cases(BASEFEE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn blockhash() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_BLOCKHASH.len()];
            v.extend(&HOST_BLOCKHASH);
            v
        })];

        test_op_cases(BLOCKHASH, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn coinbase() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_COINBASE.len()];
            v.extend(&HOST_COINBASE);
            v
        })];

        test_op_cases(COINBASE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn gasprice() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_GASPRICE.len()];
            v.extend(&HOST_ENV_GASPRICE);
            v
        })];

        test_op_cases(GASPRICE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn origin() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_ORIGIN.len()];
            v.extend(&HOST_ENV_ORIGIN);
            v
        })];

        test_op_cases(ORIGIN, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn gaslimit() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_GASLIMIT.len()];
            v.extend(&HOST_GASLIMIT);
            v
        })];

        test_op_cases(GASLIMIT, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn number() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_NUMBER.len()];
            v.extend(&HOST_NUMBER);
            v
        })];

        test_op_cases(NUMBER, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn timestamp() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_TIMESTAMP.len()];
            v.extend(&HOST_TIMESTAMP);
            v
        })];

        test_op_cases(TIMESTAMP, None, &cases, Some(-1), ResultLocation::Stack);
    }

    // TODO
    #[test]
    fn sload() {
        let cases = [Case::Args1((
            x("0000000000000000000000000000000000000000000000000000000000000000"),
            x("0000000000000000000000000000000000000000000000000000000000000000"),
        ))];

        test_op_cases(SLOAD, None, &cases, Some(-1), ResultLocation::Stack);
    }

    // TODO
    #[ignore]
    #[test]
    fn sstore() {
        let cases = [Case::Args2((
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000000"),
        ))];

        test_op_cases(SSTORE, None, &cases, Some(-1), ResultLocation::Stack);
    }

    // TODO
    #[ignore]
    #[test]
    fn tstore() {
        let cases = [Case::Args2((
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000000"),
        ))];

        test_op_cases(TSTORE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn ret() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(MSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                x("0000000000000000000000000000000000000e0d0c0b0a090807060504030201"),
                vec![],
            )),
        ));
        let cases = [Case::Args2((
            x("0000000000000000000000000000000000000000000000000000000000000012"),
            x("000000000000000000000000000000000000000000000000000000000000000e"),
            x("0e0d0c0b0a090807060504030201"),
        ))];

        test_op_cases(
            RETURN,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Output(0),
        );
    }
    #[test]
    fn gas() {
        let cases = [Case::Args0(x(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ))];

        test_op_cases(GAS, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn difficulty() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_DIFFICULTY.len()];
            v.extend(&HOST_ENV_DIFFICULTY);
            v
        })];

        test_op_cases(DIFFICULTY, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn blobbasefee() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_BLOBBASEFEE.len()];
            v.extend(&HOST_ENV_BLOBBASEFEE);
            v
        })];

        test_op_cases(BLOBBASEFEE, None, &cases, Some(-1), ResultLocation::Stack);
    }
    #[test]
    fn blobhash() {
        let cases = [
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                HOST_ENV_BLOB_HASHES[0].to_vec(),
            )),
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                HOST_ENV_BLOB_HASHES[1].to_vec(),
            )),
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                HOST_ENV_BLOB_HASHES[2].to_vec(),
            )),
            Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                vec![0; 32],
            )),
        ];

        test_op_cases(BLOBHASH, None, &cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn pop() {
        let cases = [Case::Args1((
            x("123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"),
            x(""),
        ))];

        test_op_cases(POP, None, &cases, Some(0), ResultLocation::Stack);
    }

    #[test]
    fn dup() {
        let mut res_mem = vec![];
        let mut preamble_bytecode = vec![];
        let mut val = [0u8; 32];
        let val_last_idx = val.len() - 1;

        for dup_case in 1..=2 {
            preamble_bytecode.clear();
            res_mem.clear();

            val[val_last_idx] = 1;
            res_mem.extend(&val);
            for v in 1..=dup_case {
                val[val_last_idx] = v;

                preamble_bytecode.push(PUSH32);
                preamble_bytecode.extend(&val);
            }
            for v in (1..=dup_case).rev() {
                val[val_last_idx] = v;
                res_mem.extend(&val);
            }
            let cases = [Case::Args0(res_mem.clone())];

            let op = match dup_case {
                1 => DUP1,
                2 => DUP2,
                _ => {
                    panic!("unsupported DUP{}", dup_case)
                }
            };

            test_op_cases(
                op,
                Some(&preamble_bytecode),
                &cases,
                Some(-1),
                ResultLocation::Stack,
            );
        }
    }

    #[test]
    fn swap() {
        let mut res_mem = vec![];
        let mut preamble_bytecode = vec![];
        let mut val = [0u8; 32];
        let val_last_idx = val.len() - 1;

        for swap_case in 1..=2 {
            preamble_bytecode.clear();
            res_mem.clear();

            for i in 0..=swap_case {
                let v = i + 1;
                val[val_last_idx] = v;

                preamble_bytecode.push(PUSH32);
                preamble_bytecode.extend(&val);
            }
            for i in (0..=swap_case).rev() {
                let v = if i == 0 {
                    swap_case + 1
                } else if i == swap_case {
                    1
                } else {
                    i + 1
                };
                val[val_last_idx] = v;
                res_mem.extend(&val);
            }
            let cases = [Case::Args0(res_mem.clone())];

            let op = match swap_case {
                1 => SWAP1,
                2 => SWAP2,
                _ => {
                    panic!("unsupported SWAP{}", swap_case)
                }
            };

            test_op_cases(
                op,
                Some(&preamble_bytecode),
                &cases,
                Some(-1),
                ResultLocation::Stack,
            );
        }
    }

    #[test]
    fn jump() {
        let mut preamble_bytecode = vec![];

        // current offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMP),
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000043"), // 67
                vec![],
            )),
        ));

        // current offset: + 32 (ARGS for PUSH) + 1 (PUSH32) + 1 (JUMP) = 34

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"), // 7
                vec![],
            )),
        ));

        // current offset: 34 + 32 (ARGS for PUSH) + 1 (PUSH32) = 67

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a"),
        ))];

        test_op_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(32),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn jumpi_do_not_jump() {
        let mut preamble_bytecode = vec![];

        // current offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMPI),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000064"), // 100
                x("0000000000000000000000000000000000000000000000000000000000000000"), // skip this jump
                vec![],
            )),
        ));

        // current offset: + 32*2 (ARGS for PUSH) + 2 (PUSH32) + 1 (JUMP) = 67

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"), // 7
                vec![],
            )),
        ));

        // current offset: 67 + 32 (ARGS for PUSH) + 1 (PUSH32) = 100

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000007"),
        ))];

        test_op_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(EVM_WORD_BYTES as i32 * 2),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn jumpi_do_jump() {
        let mut preamble_bytecode = vec![];

        // current offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMPI),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000064"), // 100
                x("0000000000000000000000000000000000000000000000000000000000000003"), // do not skip this jump
                vec![],
            )),
        ));

        // current offset: + 32*2 (ARGS for PUSH) + 2 (PUSH32) + 1 (JUMP) = 67

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                vec![],
            )),
        ));

        // current offset: 67 + 32 (ARGS for PUSH) + 1 (PUSH32) = 100

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a"),
        ))];

        test_op_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(EVM_WORD_BYTES as i32 * 1),
            ResultLocation::Stack,
        );
    }

    #[test]
    fn compound_or() {
        let mut preamble = vec![];
        // 1|0=1
        preamble.extend(compile_op_with_args_bytecode(
            Some(OR),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000000"),
                vec![],
            )),
        ));
        // 1|2=3
        let cases = &[Case::Args1((
            x("0000000000000000000000000000000000000000000000000000000000000002"),
            x("0000000000000000000000000000000000000000000000000000000000000003"),
        ))];

        test_op_cases(OR, Some(&preamble), cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn compound_add() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(ADD),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                vec![],
            )),
        ));
        preamble.extend(compile_op_with_args_bytecode(
            Some(ADD),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                x("0000000000000000000000000000000000000000000000000000000000000004"),
                vec![],
            )),
        ));
        preamble.push(ADD);
        let cases = &[Case::Args1((
            x("0000000000000000000000000000000000000000000000000000000000000005"),
            x("000000000000000000000000000000000000000000000000000000000000000f"),
        ))];

        test_op_cases(ADD, Some(&preamble), cases, Some(-1), ResultLocation::Stack);
    }

    #[test]
    fn compound_mul_add_div() {
        let mut preamble = vec![];
        // 2*3=6
        preamble.extend(compile_op_with_args_bytecode(
            Some(MUL),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                x("0000000000000000000000000000000000000000000000000000000000000003"),
                vec![],
            )),
        ));
        //6+7=13
        preamble.extend(compile_op_with_args_bytecode(
            Some(ADD),
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"),
                vec![],
            )),
        ));

        //27/13=2
        let cases = &[Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000001b"),
            x("0000000000000000000000000000000000000000000000000000000000000002"),
        ))];

        test_op_cases(DIV, Some(&preamble), cases, Some(-1), ResultLocation::Stack);
    }
}
