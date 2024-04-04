#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        consts::SP_BASE_MEM_OFFSET_DEFAULT,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{
                ADD,
                ADDMOD,
                ADDRESS,
                AND,
                BALANCE,
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
                CODECOPY,
                CODESIZE,
                COINBASE,
                CREATE,
                CREATE2,
                DIFFICULTY,
                DIV,
                DUP1,
                EQ,
                EXP,
                EXTCODECOPY,
                EXTCODEHASH,
                EXTCODESIZE,
                GAS,
                GASLIMIT,
                GASPRICE,
                GT,
                ISZERO,
                JUMP,
                JUMPDEST,
                JUMPI,
                KECCAK256,
                LOG0,
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
                PC,
                POP,
                PUSH32,
                RETURN,
                SAR,
                SDIV,
                SELFBALANCE,
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
                TIMESTAMP,
                TLOAD,
                TSTORE,
                XOR,
            },
        },
        utilities::EVM_WORD_BYTES,
    };
    use alloc::{rc::Rc, string::ToString, vec, vec::Vec};
    use alloy_primitives::{hex, Bytes, B256};
    use core::cell::RefCell;
    use fluentbase_codec::Encoder;
    use fluentbase_runtime::{ExecutionResult, Runtime, RuntimeContext};
    use fluentbase_sdk::evm::{Address, ContractInput, U256};
    use fluentbase_types::{Account, AccountDb, InMemoryAccountDb};
    use lazy_static::lazy_static;
    use log::debug;
    use rwasm::engine::bytecode::Instruction;
    use rwasm_codegen::{BinaryFormat, BinaryFormatWriter, InstructionSet, ReducedModule};

    static CONTRACT_ADDRESS: [u8; 20] = [1; 20]; // Address - 20 bytes
    static CONTRACT_CALLER: [u8; 20] = [2; 20]; // Address - 20 bytes
    static CONTRACT_VALUE: [u8; 32] = [3; 32]; // U256 - 32 bytes
    static SYSTEM_CODESIZE: [u8; 4] = [4; 4]; // u32 - 4 bytes
    static CONTRACT_INPUT: &[u8] = &[
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
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
    static CONTRACT_BYTECODE: &[u8] = &[
        70, 80, 74, 157, 210, 30, 193, 224, 195, 186, 69, 157, 108, 59, 26, 225, 149, 85, 152, 222,
    ];

    lazy_static! {
        static ref SALT1: B256 = B256::left_padding_from(&[
            209, 102, 91, 188, 156, 121, 93, 221, 156, 57, 106, 55, 154, 62, 20, 23, 21, 110, 34,
            225, 120, 134, 226, 58, 173, 164, 80, 17, 180, 3, 29, 194
        ]);
        static ref CALLER_ADDRESS: Address = Address::new([
            187, 183, 200, 54, 217, 36, 224, 17, 157, 249, 177, 246, 95, 46, 125, 23, 141, 21, 7,
            19
        ]);
        static ref CALLER_ACCOUNT_BYTECODE: Bytes = Bytes::copy_from_slice(&[
            88, 157, 82, 216, 126, 78, 124, 162, 207, 40, 234, 212, 165, 125, 154, 140, 100, 201,
            144, 135, 94
        ]);
        static ref CALLER_ACCOUNT: Account = Account {
            balance: U256::from_be_slice(&[
                211, 109, 149, 183, 251, 106, 161, 179, 235, 43, 144, 130, 230, 246, 88, 56, 246,
                227, 21, 13, 248, 150, 47, 216, 35, 83, 200, 200, 198, 238, 186, 27
            ]),
            nonce: 3,
            code_hash: B256::new(
                keccak_hash::keccak(CALLER_ACCOUNT_BYTECODE.clone()).to_fixed_bytes()
            ),
            code: Some(CALLER_ACCOUNT_BYTECODE.clone()),
        };
        static ref USER1_ADDRESS: Address = Address::new([
            4, 161, 101, 36, 210, 18, 205, 56, 117, 142, 62, 212, 144, 140, 203, 201, 68, 42, 97,
            35
        ]);
        static ref USER1_ACCOUNT_BYTECODE: Bytes = Bytes::copy_from_slice(&[
            26, 174, 195, 56, 172, 103, 71, 218, 84, 75, 219, 13, 115, 160
        ]);
        static ref USER1_ACCOUNT: Account = Account {
            balance: U256::from_be_slice(&[
                151, 241, 205, 52, 210, 166, 83, 156, 175, 189, 213, 173, 77, 189, 54, 63, 181,
                190, 236, 192, 109, 8, 118, 183, 212, 204, 45, 126, 92, 117, 116, 9
            ]),
            nonce: 4,
            code_hash: B256::new(
                keccak_hash::keccak(USER1_ACCOUNT_BYTECODE.clone()).to_fixed_bytes()
            ),
            code: Some(USER1_ACCOUNT_BYTECODE.clone()),
        };
        static ref USER2_ADDRESS: Address = Address::new([
            229, 162, 19, 139, 55, 179, 49, 233, 27, 118, 237, 201, 79, 246, 204, 24, 202, 220, 10,
            27
        ]);
        static ref USER2_ACCOUNT_BYTECODE: Bytes =
            Bytes::copy_from_slice(&[154, 231, 160, 31, 70, 70, 101, 234, 71, 146, 137, 166]);
        static ref USER2_ACCOUNT: Account = Account {
            balance: U256::from_be_slice(&[
                229, 193, 13, 6, 93, 29, 230, 94, 55, 147, 173, 187, 196, 7, 33, 86, 254, 115, 65,
                90, 35, 130, 74, 205, 247, 176, 133, 11, 208, 88, 79, 163
            ]),
            nonce: 5,
            code_hash: B256::new(
                keccak_hash::keccak(USER2_ACCOUNT_BYTECODE.clone()).to_fixed_bytes()
            ),
            code: Some(USER2_ACCOUNT_BYTECODE.clone()),
        };
    }

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
        Args4((Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)),
        // result_expected param_1..param_n
        Universal(Vec<Vec<u8>>),
    }

    #[derive(Clone)]
    enum ResultLocation {
        Stack,
        Memory(usize),
        Output(usize),
        Private,
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
            Case::Args4(args) => {
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.3.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.2.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.1.clone());
                evm_bytecode.push(PUSH32);
                evm_bytecode.extend(args.0.clone());
            }
            Case::Universal(args) => {
                for i in (0..args.len() - 1).rev() {
                    evm_bytecode.push(PUSH32);
                    evm_bytecode.extend(args[i].clone());
                }
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

    fn test_cases(
        opcode: u8,
        bytecode_preamble: Option<&[u8]>,
        cases: &[Case],
        sp: Option<i32>,
        result_location: ResultLocation,
        instruction_result: Option<InstructionResult>,
    ) {
        for case in cases {
            let res_expected = match case {
                Case::Args0(v) => v.clone(),
                Case::Args1(v) => v.1.clone(),
                Case::Args2(v) => v.2.clone(),
                Case::Args3(v) => v.3.clone(),
                Case::Args4(v) => v.4.clone(),
                Case::Universal(v) => {
                    let i = v.len() - 1;
                    v[i].clone()
                }
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
                instruction_result,
            );
        }
    }

    fn test_op(
        opcode: u8,
        evm_bytecode: Vec<u8>,
        res_expected: Vec<u8>,
        sp: Option<i32>,
        result_location: ResultLocation,
        instruction_result: Option<InstructionResult>,
    ) {
        let res = run_test(&evm_bytecode, instruction_result);
        if res == None {
            return;
        }
        let (mut global_memory, output) = res.unwrap();
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
            ResultLocation::Private => {}
        }
    }

    fn run_test(
        evm_bytecode_bytes: &Vec<u8>,
        instruction_result: Option<InstructionResult>,
    ) -> Option<(Vec<u8>, Vec<u8>)> {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let import_linker = Runtime::<()>::new_sovereign_linker();
        let mut compiler = EvmCompiler::new(&import_linker, false, evm_binary.as_ref());

        let res = compiler.run(None, None);
        if let Some(instruction_result) = instruction_result {
            assert_eq!(res, instruction_result);
            return None;
        } else {
            if !res.is_ok() {
                debug!("instruction_result: {:?}", res);
            }
            assert!(res.is_ok());
        }

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

        let mut contract_input = ContractInput::default();
        contract_input.contract_address = Address::new(CONTRACT_ADDRESS);
        contract_input.contract_caller = Address::new(CONTRACT_CALLER);
        contract_input.contract_value = U256::from_be_bytes(CONTRACT_VALUE);
        contract_input.contract_code_size = u32::from_be_bytes(SYSTEM_CODESIZE);
        contract_input.contract_input = Bytes::from(CONTRACT_INPUT);
        contract_input.contract_input_size = CONTRACT_INPUT.len() as u32;
        contract_input.env_chain_id = u64::from_be_bytes(HOST_CHAINID);
        contract_input.block_base_fee = U256::from_be_bytes(HOST_BASEFEE);
        contract_input.block_hash = B256::new(HOST_BLOCKHASH);
        contract_input.block_coinbase = Address::from(HOST_COINBASE);
        contract_input.block_gas_limit = u64::from_be_bytes(HOST_GASLIMIT);
        contract_input.block_number = u64::from_be_bytes(HOST_NUMBER);
        contract_input.block_timestamp = u64::from_be_bytes(HOST_TIMESTAMP);
        contract_input.block_difficulty = u64::from_be_bytes(HOST_ENV_DIFFICULTY);
        contract_input.contract_bytecode = Bytes::copy_from_slice(CONTRACT_BYTECODE);
        // contract_input.tx_blob_gas_price = u64::from_be_bytes(HOST_ENV_BLOBBASEFEE);
        contract_input.tx_gas_price = U256::from_be_bytes(HOST_ENV_GASPRICE);
        contract_input.tx_caller = Address::new(HOST_ENV_ORIGIN);
        // contract_input.tx_blob_hashes = HOST_ENV_BLOB_HASHES
        //     .iter()
        //     .map(|v| B256::from_slice(v))
        //     .collect();
        let mut account_db = InMemoryAccountDb::default();

        account_db.update_account(&CALLER_ADDRESS, &CALLER_ACCOUNT);
        account_db.update_account(&USER1_ADDRESS, &USER1_ACCOUNT);
        account_db.update_account(&USER2_ADDRESS, &USER2_ACCOUNT);
        let ci = contract_input.encode_to_vec(0);
        runtime_ctx = runtime_ctx
            .with_input(ci)
            .with_fuel_limit(10_000_000)
            .with_account_db(Rc::new(RefCell::new(account_db)))
            .with_caller(CALLER_ADDRESS.clone());

        let runtime = Runtime::new(runtime_ctx, &import_linker);
        let mut runtime = runtime.unwrap();
        let result = runtime.call();
        assert!(result.is_ok());
        let execution_result: ExecutionResult<()> = result.unwrap();
        for (idx, log) in execution_result.tracer().logs.iter().enumerate() {
            let memory_changes = &log.memory_changes;
            let stack = &log.stack;
            let prev_opcode = if idx > 0 {
                Some(execution_result.tracer().logs[idx - 1].opcode)
            } else {
                None
            };
            debug!(
                "pc:{:?} idx:{} op:{:?} (prev:{:?}) memchz(for prev):{:?} stack:{:?}",
                log.program_counter,
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
        let execution_result_data = execution_result.data();
        let execution_result_exit_code = execution_result_data.exit_code();
        assert_eq!(execution_result_exit_code, 0);

        // debug!(
        //     "\nglobal_memory ({}): {:?}\n",
        //     global_memory_len, &global_memory
        // );

        Some((global_memory, runtime.data().output().clone()))
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

        test_cases(
            EQ,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            ISZERO,
            None,
            &cases.map(|v| Case::Args1(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            NOT,
            None,
            &cases.map(|v| Case::Args1(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SHL,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SHR,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            BYTE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            LT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SLT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            GT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SGT,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SAR,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            ADD,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SUB,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            ADDMOD,
            None,
            &cases.map(|v| Case::Args3(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SIGNEXTEND,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
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

        test_cases(
            MUL,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            MULMOD,
            None,
            &cases.map(|v| Case::Args3(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            EXP,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            DIV,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SDIV,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            AND,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            MOD,
            None,
            &cases.map(|v| Case::Args2(v)),
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            SMOD,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn or() {
        let cases = [Case::Args2((
            x("0x00003100080000300f0000070000a000c0000000000030001000200030000001"),
            x("0x000003000400040000000a010000b000a0000000f000000007000004200a0001"),
            x("0x000033000c0004300f000a070000b000e0000000f000300017002004300a0001"),
        ))];

        test_cases(
            OR,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            XOR,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            MSTORE,
            None,
            &cases,
            Some(0),
            ResultLocation::Memory(0),
            None,
        );
    }

    #[test]
    fn mstore8() {
        let cases = [
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000000"), // offset
                x("000000000f00000100000000f0000000100000000f00000000000000000f0032"), // data
                x("3200000000000000000000000000000000000000000000000000000000000000"), // result
            )),
            Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000008"),
                x("000000000f00000100000000f0000000100000000f00000000000000000f00af"),
                x("0000000000000000af0000000000000000000000000000000000000000000000"),
            )),
        ];

        test_cases(
            MSTORE8,
            None,
            &cases,
            Some(0),
            ResultLocation::Memory(0),
            None,
        );
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

        test_cases(
            MLOAD,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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
            "0000000000000000000000000000000000000000000000000000000000000011",
        ))];

        test_cases(
            MSIZE,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
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

        test_cases(
            MCOPY,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Memory(0),
            None,
        );
    }

    #[test]
    fn caller() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - CONTRACT_ADDRESS.len()];
            v.extend(&CONTRACT_CALLER);
            v
        })];

        test_cases(
            CALLER,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn address() {
        let cases = [Case::Args0(
            B256::left_padding_from(CONTRACT_ADDRESS.as_slice()).to_vec(),
        )];

        test_cases(
            ADDRESS,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore]
    #[test]
    fn log0() {
        let mut preamble = vec![];
        let log_data: Vec<u8> = vec![
            169, 83, 152, 138, 79, 114, 66, 48, 158, 239, 14, 15, 55, 215, 227, 148, 175, 166, 208,
            143, 171, 188, 10, 119, 66, 39, 108, 239, 49, 6, 148, 173, 103, 45, 42, 121, 55, 216,
            254, 91, 168, 44,
        ];
        let log_data_mem_offset: u32 = 9;
        let log_data_evm_words_size = (log_data.len() + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES;
        for (idx, chunk) in log_data.chunks(EVM_WORD_BYTES).enumerate() {
            preamble.extend(compile_op_with_args_bytecode(
                Some(MSTORE),
                &Case::Args2((
                    B256::left_padding_from(
                        &(log_data_mem_offset as usize + idx * EVM_WORD_BYTES).to_be_bytes(),
                    )
                    .to_vec(),
                    B256::left_padding_from(chunk).to_vec(),
                    vec![],
                )),
            ));
        }
        let cases = [Case::Args2((
            B256::left_padding_from(&log_data_mem_offset.to_be_bytes()).to_vec(), // mem_offset
            B256::left_padding_from(&log_data_evm_words_size.to_be_bytes()).to_vec(), // size
            vec![],
        ))];

        // TODO extract data from storage and compare with data above

        test_cases(
            LOG0,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Private,
            None,
        );
    }

    #[test]
    fn balance() {
        let cases = [
            Case::Args1((
                B256::left_padding_from(CALLER_ADDRESS.as_slice()).to_vec(),
                CALLER_ACCOUNT.balance.to_be_bytes_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER1_ADDRESS.as_slice()).to_vec(),
                USER1_ACCOUNT.balance.to_be_bytes_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER2_ADDRESS.as_slice()).to_vec(),
                USER2_ACCOUNT.balance.to_be_bytes_vec(),
            )),
        ];

        test_cases(
            BALANCE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore]
    #[test]
    fn create() {
        let mut preamble = vec![];

        // valid rwasm bytecode
        let init_bytecode_data = vec![
            61, 0, 0, 0, 0, 0, 0, 0, 2, 4, 0, 0, 0, 0, 0, 0, 0, 3, 11, 0, 0, 0, 0, 0, 0, 0, 0, 9,
            0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0,
            0, 0, 0, 0, 0, 4, 2, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0,
            0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 3, 2, 0, 0, 0, 0, 0, 0, 0, 1, 13, 0, 0, 0, 0, 0, 0,
            0, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 61, 0, 0, 0, 0, 0, 16, 0, 0, 61, 0, 0, 0, 0, 0, 0, 0,
            12, 13, 0, 0, 0, 0, 0, 0, 0, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0,
            1, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 4, 2, 0,
            0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0,
            0, 0, 0, 0, 3, 2, 0, 0, 0, 0, 0, 0, 0, 1, 13, 0, 0, 0, 0, 0, 0, 0, 5, 5, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];

        // valid rwasm bytecode
        // let init_bytecode_data =
        //     include_bytes!("../../code-snippets/bin/other_deploy_contract_test.rwasm").
        // to_vec();

        let deploy_bytecode_mem_offset: u32 = 9;
        for (idx, chunk) in init_bytecode_data.chunks(1).enumerate() {
            preamble.extend(compile_op_with_args_bytecode(
                Some(MSTORE8),
                &Case::Args2((
                    B256::left_padding_from(
                        &(deploy_bytecode_mem_offset as usize + idx).to_be_bytes(),
                    )
                    .to_vec(),
                    B256::left_padding_from(chunk).to_vec(),
                    vec![],
                )),
            ));
        }

        let deployed_contract_address = CALLER_ADDRESS.create(CALLER_ACCOUNT.nonce);

        let cases = [Case::Args3((
            B256::left_padding_from(&[0]).to_vec(), // value TODO set some not zero value
            B256::left_padding_from(&deploy_bytecode_mem_offset.to_be_bytes()).to_vec(), // offset
            B256::left_padding_from(&init_bytecode_data.len().to_be_bytes()).to_vec(), /* size */
            B256::left_padding_from(deployed_contract_address.as_slice()).to_vec(), /* result address */
        ))];

        test_cases(
            CREATE,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore]
    #[test]
    fn create2() {
        let mut preamble = vec![];

        // valid rwasm bytecode
        let init_bytecode_data = vec![
            1, 0, 0, 0, 0, 0, 0, 0, 3, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 4, 2, 0,
            0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0,
            0, 0, 0, 0, 3, 2, 0, 0, 0, 0, 0, 0, 0, 1, 13, 0, 0, 0, 0, 0, 0, 0, 5, 5, 0, 0, 0, 0, 0,
            0, 0, 0, 61, 0, 0, 0, 0, 0, 16, 0, 0, 61, 0, 0, 0, 0, 0, 0, 0, 12, 13, 0, 0, 0, 0, 0,
            0, 0, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
            3, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 4, 2, 0, 0, 0, 0, 0, 0, 0, 1, 1,
            0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 3, 2, 0, 0,
            0, 0, 0, 0, 0, 1, 13, 0, 0, 0, 0, 0, 0, 0, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let deploy_bytecode_mem_offset: u32 = 9;
        for (idx, chunk) in init_bytecode_data.chunks(1).enumerate() {
            preamble.extend(compile_op_with_args_bytecode(
                Some(MSTORE8),
                &Case::Args2((
                    B256::left_padding_from(
                        &(deploy_bytecode_mem_offset as usize + idx).to_be_bytes(),
                    )
                    .to_vec(),
                    B256::left_padding_from(chunk).to_vec(),
                    vec![],
                )),
            ));
        }

        let init_bytecode_hash = keccak_hash::keccak(&init_bytecode_data);
        let deployed_contract_address =
            CALLER_ADDRESS.create2(SALT1.clone(), init_bytecode_hash.as_fixed_bytes());

        let cases = [Case::Args4((
            B256::left_padding_from(&[0]).to_vec(), // value TODO set some not zero value
            B256::left_padding_from(&deploy_bytecode_mem_offset.to_be_bytes()).to_vec(), // offset
            B256::left_padding_from(&init_bytecode_data.len().to_be_bytes()).to_vec(), // size
            SALT1.to_vec(),                         // salt
            B256::left_padding_from(deployed_contract_address.as_slice()).to_vec(), /* result address */
        ))];

        test_cases(
            CREATE2,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn extcodesize() {
        let cases = [
            Case::Args1((
                B256::left_padding_from(CALLER_ADDRESS.as_slice()).to_vec(),
                B256::left_padding_from(CALLER_ACCOUNT_BYTECODE.len().to_be_bytes().as_slice())
                    .to_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER1_ADDRESS.as_slice()).to_vec(),
                B256::left_padding_from(USER1_ACCOUNT_BYTECODE.len().to_be_bytes().as_slice())
                    .to_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER2_ADDRESS.as_slice()).to_vec(),
                B256::left_padding_from(USER2_ACCOUNT_BYTECODE.len().to_be_bytes().as_slice())
                    .to_vec(),
            )),
        ];

        test_cases(
            EXTCODESIZE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn extcodehash() {
        let cases = [
            Case::Args1((
                B256::left_padding_from(CALLER_ADDRESS.as_slice()).to_vec(),
                CALLER_ACCOUNT.code_hash.to_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER1_ADDRESS.as_slice()).to_vec(),
                USER1_ACCOUNT.code_hash.to_vec(),
            )),
            Case::Args1((
                B256::left_padding_from(USER2_ADDRESS.as_slice()).to_vec(),
                USER2_ACCOUNT.code_hash.to_vec(),
            )),
        ];

        test_cases(
            EXTCODEHASH,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn extcodecopy() {
        let mut bytecode_memory_result: Vec<u8> = vec![];
        let bytecode_offset = 3;
        let dest_offset: u32 = 9;
        let size = USER2_ACCOUNT_BYTECODE.len() + 2 - bytecode_offset;
        bytecode_memory_result.extend_from_slice(&[0, 0]);
        let mut bytecode_offset_tail_expected = bytecode_offset + size;
        let mut bytecode_offset_tail_fact =
            if bytecode_offset_tail_expected > USER2_ACCOUNT_BYTECODE.len() {
                USER2_ACCOUNT_BYTECODE.len()
            } else {
                bytecode_offset_tail_expected
            };
        bytecode_memory_result
            .extend_from_slice(&USER2_ACCOUNT_BYTECODE[bytecode_offset..bytecode_offset_tail_fact]);
        bytecode_memory_result.extend_from_slice(&[0, 0]);
        let bytecode_memory_result_len = bytecode_memory_result.len();

        let mut preamble = vec![];
        let total_words_to_mstore =
            (bytecode_memory_result_len + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES;
        for wc in 0..total_words_to_mstore {
            preamble.extend(compile_op_with_args_bytecode(
                Some(MSTORE),
                &Case::Args2((
                    {
                        let mem_offset = dest_offset as usize + wc * EVM_WORD_BYTES;
                        let v = B256::left_padding_from(&mem_offset.to_be_bytes());
                        v.to_vec()
                    },
                    x("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                    vec![],
                )),
            ));
        }

        let cases = [Case::Args4((
            B256::left_padding_from(USER2_ADDRESS.as_slice()).to_vec(), // address
            B256::left_padding_from(&dest_offset.to_be_bytes()).to_vec(), // dest_offset
            B256::left_padding_from(&bytecode_offset.to_be_bytes()).to_vec(), // offset
            B256::left_padding_from(&size.to_be_bytes()).to_vec(),      //size
            bytecode_memory_result,
        ))];

        test_cases(
            EXTCODECOPY,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Memory(7),
            None,
        );
    }

    #[test]
    fn selfbalance() {
        let cases = [Case::Args0(CALLER_ACCOUNT.balance.to_be_bytes_vec())];

        test_cases(
            SELFBALANCE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn codecopy() {
        let mut contract_bytecode_memory_result: Vec<u8> = vec![];
        let bytecode_offset = 3;
        let dest_offset: u32 = 9;
        let size = CONTRACT_BYTECODE.len() + 2 - bytecode_offset;
        contract_bytecode_memory_result.extend_from_slice(&[0, 0]);
        let mut bytecode_offset_tail_expected = bytecode_offset + size;
        let mut bytecode_offset_tail_fact =
            if bytecode_offset_tail_expected > CONTRACT_BYTECODE.len() {
                CONTRACT_BYTECODE.len()
            } else {
                bytecode_offset_tail_expected
            };
        contract_bytecode_memory_result
            .extend_from_slice(&CONTRACT_BYTECODE[bytecode_offset..bytecode_offset_tail_fact]);
        contract_bytecode_memory_result.extend_from_slice(&[0, 0]);
        let contract_bytecode_memory_result_len = contract_bytecode_memory_result.len();

        let mut preamble = vec![];
        let total_words_to_mstore =
            (contract_bytecode_memory_result_len + EVM_WORD_BYTES - 1) / EVM_WORD_BYTES;
        for wc in 0..total_words_to_mstore {
            preamble.extend(compile_op_with_args_bytecode(
                Some(MSTORE),
                &Case::Args2((
                    {
                        let mem_offset = dest_offset as usize + wc * EVM_WORD_BYTES;
                        let v = B256::left_padding_from(&mem_offset.to_be_bytes());
                        v.to_vec()
                    },
                    x("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                    vec![],
                )),
            ));
        }

        let cases = [Case::Args3((
            {
                // dest offset
                let v = B256::left_padding_from(&dest_offset.to_be_bytes());
                v.to_vec()
            },
            B256::left_padding_from(&bytecode_offset.to_be_bytes()).to_vec(),
            B256::left_padding_from(&size.to_be_bytes()).to_vec(),
            contract_bytecode_memory_result,
        ))];

        test_cases(
            CODECOPY,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Memory(7),
            None,
        );
    }

    #[test]
    fn calldatasize() {
        let mut cases = vec![];
        let len_be = CONTRACT_INPUT.len().to_be_bytes();
        let mut v = [0u8; 32];
        v[32 - len_be.len()..].copy_from_slice(&len_be);
        cases.push(Case::Args0(v.to_vec()));

        test_cases(
            CALLDATASIZE,
            None,
            &cases,
            Some(32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            CALLDATALOAD,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn calldatacopy() {
        let cases = vec![
            Case::Args3((
                x("0000000000000000000000000000000000000000000000000000000000000000"), // dst
                x("0000000000000000000000000000000000000000000000000000000000000000"), // src
                x("0000000000000000000000000000000000000000000000000000000000000029"), // size
                x("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223242526272829"),
            )),
            Case::Args3((
                x("0000000000000000000000000000000000000000000000000000000000000000"), // dst
                x("0000000000000000000000000000000000000000000000000000000000000009"), // src
                x("0000000000000000000000000000000000000000000000000000000000000029"), // size
                x("0a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223242526272829"),
            )),
        ];

        test_cases(
            CALLDATACOPY,
            None,
            &cases,
            Some(0),
            ResultLocation::Memory(0),
            None,
        );
    }

    #[test]
    fn callvalue() {
        let cases = [Case::Args0(CONTRACT_VALUE.to_vec())];

        test_cases(
            CALLVALUE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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
            test_cases(
                KECCAK256,
                Some(&case.0),
                &[case.1.clone()],
                Some(EVM_WORD_BYTES as i32),
                ResultLocation::Stack,
                None,
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

        test_cases(
            CODESIZE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn chainid() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_CHAINID.len()];
            v.extend(&HOST_CHAINID);
            v
        })];

        test_cases(
            CHAINID,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn basefee() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_BASEFEE.len()];
            v.extend(&HOST_BASEFEE);
            v
        })];

        test_cases(
            BASEFEE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn blockhash() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_BLOCKHASH.len()];
            v.extend(&HOST_BLOCKHASH);
            v
        })];

        test_cases(
            BLOCKHASH,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn coinbase() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_COINBASE.len()];
            v.extend(&HOST_COINBASE);
            v
        })];

        test_cases(
            COINBASE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn gasprice() {
        let cases = [Case::Args0({
            let mut v = vec![0; EVM_WORD_BYTES - HOST_ENV_GASPRICE.len()];
            v.extend(&HOST_ENV_GASPRICE);
            v
        })];

        test_cases(
            GASPRICE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn origin() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_ORIGIN.len()];
            v.extend(&HOST_ENV_ORIGIN);
            v
        })];

        test_cases(
            ORIGIN,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn gaslimit() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_GASLIMIT.len()];
            v.extend(&HOST_GASLIMIT);
            v
        })];

        test_cases(
            GASLIMIT,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn number() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_NUMBER.len()];
            v.extend(&HOST_NUMBER);
            v
        })];

        test_cases(
            NUMBER,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn timestamp() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_TIMESTAMP.len()];
            v.extend(&HOST_TIMESTAMP);
            v
        })];

        test_cases(
            TIMESTAMP,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore] // TODO no implementation yet
    #[test]
    fn sstore_sload() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(SSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                vec![],
            )),
        ));
        let cases = [Case::Args1((
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000002"),
        ))];

        test_cases(
            SLOAD,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn tstore_tload() {
        let mut preamble = vec![];
        preamble.extend(compile_op_with_args_bytecode(
            Some(TSTORE),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000001"),
                x("0000000000000000000000000000000000000000000000000000000000000002"),
                vec![],
            )),
        ));
        let cases = [Case::Args1((
            x("0000000000000000000000000000000000000000000000000000000000000001"),
            x("0000000000000000000000000000000000000000000000000000000000000002"),
        ))];

        test_cases(
            TLOAD,
            Some(&preamble),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            RETURN,
            Some(&preamble),
            &cases,
            Some(0),
            ResultLocation::Output(0),
            None,
        );
    }
    #[test]
    fn gas() {
        let cases = [Case::Args0(x(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ))];

        test_cases(
            GAS,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
    #[test]
    fn difficulty() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_DIFFICULTY.len()];
            v.extend(&HOST_ENV_DIFFICULTY);
            v
        })];

        test_cases(
            DIFFICULTY,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore] // TODO no implementation yet
    #[test]
    fn blobbasefee() {
        let cases = [Case::Args0({
            let mut v = vec![0; 32 - HOST_ENV_BLOBBASEFEE.len()];
            v.extend(&HOST_ENV_BLOBBASEFEE);
            v
        })];

        test_cases(
            BLOBBASEFEE,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[ignore] // TODO no implementation yet
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

        test_cases(
            BLOBHASH,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn pop() {
        let cases = [Case::Args1((
            x("123456789abcdef123456789abcdef123456789abcdef123456789abcdef1234"),
            x(""),
        ))];

        test_cases(POP, None, &cases, Some(0), ResultLocation::Stack, None);
    }

    #[test]
    fn dup() {
        let mut res_mem = vec![];
        let mut preamble_bytecode = vec![];
        let mut val = [0u8; 32];
        let val_last_idx = val.len() - 1;

        for dup_case in 1..=16 {
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

            let op = DUP1 + dup_case - 1;

            test_cases(
                op,
                Some(&preamble_bytecode),
                &cases,
                Some(EVM_WORD_BYTES as i32 * (dup_case as i32 + 1)),
                ResultLocation::Stack,
                None,
            );
        }
    }

    #[test]
    fn swap() {
        let mut res_mem = vec![];
        let mut preamble_bytecode = vec![];
        let mut val = [0u8; 32];
        let val_last_idx = val.len() - 1;

        for swap_case in 1..=16 {
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

            let op = SWAP1 + swap_case - 1;

            test_cases(
                op,
                Some(&preamble_bytecode),
                &cases,
                Some(EVM_WORD_BYTES as i32 * (swap_case as i32 + 1)),
                ResultLocation::Stack,
                None,
            );
        }
    }

    #[test]
    fn jump() {
        let mut preamble_bytecode = vec![];

        // offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMP),
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000043"), // 67
                vec![],
            )),
        ));

        // offset: + 32 (ARGS for PUSH) + 1 (PUSH32) + 1 (JUMP) = 34

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"), // 7
                vec![],
            )),
        ));

        // offset: + 32 (ARGS for PUSH) + 1 (PUSH32) = 67

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMPDEST),
            &Case::Args0(vec![]),
        ));

        // offset: + 1 (JUMPDEST) = 68

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a"),
        ))];

        test_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn jump_incorrect_jumpdest() {
        let mut preamble_bytecode = vec![];

        // offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMP),
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000043"), // 67
                vec![],
            )),
        ));

        // offset: + 32 (ARGS for PUSH) + 1 (PUSH32) + 1 (JUMP) = 34

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"), // 7
                vec![],
            )),
        ));

        // offset: + 32 (ARGS for PUSH) + 1 (PUSH32) = 67

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a"),
        ))];

        test_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            Some(InstructionResult::InvalidJump),
        );
    }

    #[test]
    fn jumpi_do_not_jump() {
        let mut preamble_bytecode = vec![];

        // offset: 0

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMPI),
            &Case::Args2((
                x("0000000000000000000000000000000000000000000000000000000000000064"), // 100
                x("0000000000000000000000000000000000000000000000000000000000000000"), // skip this jump
                vec![],
            )),
        ));

        // offset: + 32*2 (ARGS for PUSH) + 2 (PUSH32) + 1 (JUMP) = 67

        // this push must be skipped
        preamble_bytecode.extend(compile_op_with_args_bytecode(
            None,
            &Case::Args1((
                x("0000000000000000000000000000000000000000000000000000000000000007"), // 7
                vec![],
            )),
        ));

        // offset: 67 + 32 (ARGS for PUSH) + 1 (PUSH32) = 100

        preamble_bytecode.extend(compile_op_with_args_bytecode(
            Some(JUMPDEST),
            &Case::Args0(vec![]),
        ));

        // offset: + 1 (JUMPDEST) = 101

        let cases = [Case::Args1((
            x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
            x("000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000007"),
        ))];

        test_cases(
            STOP, // special case
            Some(&preamble_bytecode),
            &cases,
            Some(EVM_WORD_BYTES as i32 * 2),
            ResultLocation::Stack,
            None,
        );
    }

    #[test]
    fn jumpi_do_jump() {
        let jump_decision_args = [
            x("0000000000000000 0000000000000000 0000000000000000 0000000000000001"),
            x("0000000000000000 0000000000000000 0000000000000001 0000000000000000"),
            x("0000000000000000 0000000000000001 0000000000000000 0000000000000000"),
            x("0000000000000001 0000000000000000 0000000000000000 0000000000000000"),
            x("0000000000000000 0000000000000000 0000000000000000 1000000000000000"),
            x("0000000000000000 0000000000000000 1000000000000000 0000000000000000"),
            x("0000000000000000 1000000000000000 0000000000000000 0000000000000000"),
            x("1000000000000000 0000000000000000 0000000000000000 0000000000000000"),
        ];

        for jump_decision_arg in jump_decision_args {
            let mut preamble_bytecode = vec![];

            // offset: 0

            preamble_bytecode.extend(compile_op_with_args_bytecode(
                Some(JUMPI),
                &Case::Args2((
                    x("0000000000000000000000000000000000000000000000000000000000000064"), // 100
                    jump_decision_arg, // do not skip this jump
                    vec![],
                )),
            ));

            // offset: + 32*2 (ARGS for PUSH) + 2 (PUSH32) + 1 (JUMP) = 67

            // this push must be skipped
            preamble_bytecode.extend(compile_op_with_args_bytecode(
                None,
                &Case::Args1((
                    x("0000000000000000000000000000000000000000000000000000000000000003"),
                    vec![],
                )),
            ));

            // current offset: 67 + 32 (ARGS for PUSH) + 1 (PUSH32) = 100

            preamble_bytecode.extend(compile_op_with_args_bytecode(
                Some(JUMPDEST),
                &Case::Args0(vec![]),
            ));

            // current offset: offset + 1 (JUMPDEST) = 101

            let cases = [Case::Args1((
                x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
                x("000000000000000000000000000000000000000000000000000000000000000a"),
            ))];

            test_cases(
                STOP, // special case
                Some(&preamble_bytecode),
                &cases,
                Some(EVM_WORD_BYTES as i32 * 1),
                ResultLocation::Stack,
                None,
            );
        }
    }

    #[test]
    fn jumpi_do_jump_incorrect_jumpdest() {
        let jump_decision_args = [
            x("0000000000000000 0000000000000000 0000000000000000 0000000000000001"),
            x("0000000000000000 0000000000000000 0000000000000001 0000000000000000"),
            x("0000000000000000 0000000000000001 0000000000000000 0000000000000000"),
            x("0000000000000001 0000000000000000 0000000000000000 0000000000000000"),
            x("0000000000000000 0000000000000000 0000000000000000 1000000000000000"),
            x("0000000000000000 0000000000000000 1000000000000000 0000000000000000"),
            x("0000000000000000 1000000000000000 0000000000000000 0000000000000000"),
            x("1000000000000000 0000000000000000 0000000000000000 0000000000000000"),
        ];

        for jump_decision_arg in jump_decision_args {
            let mut preamble_bytecode = vec![];

            // offset: 0

            preamble_bytecode.extend(compile_op_with_args_bytecode(
                Some(JUMPI),
                &Case::Args2((
                    x("0000000000000000000000000000000000000000000000000000000000000064"), // 100
                    jump_decision_arg, // do not skip this jump
                    vec![],
                )),
            ));

            // offset: + 32*2 (ARGS for PUSH) + 2 (PUSH32) + 1 (JUMP) = 67

            // this push must be skipped
            preamble_bytecode.extend(compile_op_with_args_bytecode(
                None,
                &Case::Args1((
                    x("0000000000000000000000000000000000000000000000000000000000000003"),
                    vec![],
                )),
            ));

            // offset: + 32 (ARGS for PUSH) + 1 (PUSH32) = 100

            let cases = [Case::Args1((
                x("000000000000000000000000000000000000000000000000000000000000000a"), // 10
                x("000000000000000000000000000000000000000000000000000000000000000a"),
            ))];

            test_cases(
                STOP, // special case
                Some(&preamble_bytecode),
                &cases,
                Some(EVM_WORD_BYTES as i32 * 1),
                ResultLocation::Stack,
                Some(InstructionResult::InvalidJump),
            );
        }
    }

    #[test]
    fn pc() {
        let cases = [Case::Args0(x(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ))];

        test_cases(
            PC,
            None,
            &cases,
            Some(EVM_WORD_BYTES as i32 * 1),
            ResultLocation::Stack,
            None,
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

        test_cases(
            OR,
            Some(&preamble),
            cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            ADD,
            Some(&preamble),
            cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
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

        test_cases(
            DIV,
            Some(&preamble),
            cases,
            Some(EVM_WORD_BYTES as i32),
            ResultLocation::Stack,
            None,
        );
    }
}
