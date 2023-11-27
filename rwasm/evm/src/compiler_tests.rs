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
        hex::decode(hex).unwrap()
    }

    use crate::translator::instructions::opcode::SHR;
    use log::debug;

    fn run_test(evm_bytecode_bytes: &Vec<u8>) -> Vec<u8> {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let mut compiler = EvmCompiler::new(evm_binary.as_ref(), false);

        compiler.instruction_set.op_i32_const(100);
        compiler.instruction_set.op_memory_grow();
        compiler.instruction_set.op_drop();

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);
        // let mut binary_format_writer_tmp = BinaryFormatWriter::new(&mut buffer_tmp);

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
            // if log.memory_changes.len() > 0 {
            //     debug!(
            //         "log opcode {} memory_changes {:?}",
            //         log.opcode, &log.memory_changes
            //     );
            // }
            for change in &log.memory_changes {
                let new_len = (change.offset + change.len) as usize;
                global_memory[change.offset as usize..new_len].copy_from_slice(&change.data);
                if new_len > global_memory_len {
                    global_memory_len = (change.offset + change.len) as usize;
                }
            }
        }
        let global_memory = global_memory[0..32].to_vec();
        debug!(
            "global_memory (len {}) {:?}",
            global_memory_len,
            &global_memory[..global_memory_len]
        );
        debug!(
            "\nexecution_result.tracer() (exit_code {}): \n{:#?}\n",
            execution_result.data().exit_code(),
            execution_result.tracer()
        );
        assert_eq!(execution_result.data().exit_code(), 0);

        global_memory
    }

    #[test]
    fn eq() {
        let cases = [
            (
                // a=-4 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=-4 b=-5 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-5 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30000 b=-30000 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=-30000 b=-30001 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30001 b=-30000 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=1 b=1 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=2 b=1 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=1 b=2 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30000 b=30000 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=30001 b=30000 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30000 b=30001 r=0
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
            (
                // shift=1 value=1 r=4
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            (
                // shift=2 value=1 r=4
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                d("0xF000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFE"),
                d("0xFF00000000000000000000000000000000000000000000000000000000000000"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x8000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000100"),
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
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
            (
                // shift=1 value=1 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // shift=2 value=4 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // shift=1 value=4 r=2
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0x00000000000000000000000000000000000000000000000000000000000000FF"),
                d("0x000000000000000000000000000000000000000000000000000000000000000F"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000004"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000008"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x00ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            ),
            (
                // external
                d("0x00000000000000000000000000000000000000000000000000000000000000ff"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000100"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0x0000000000000000000000000000000000000000000000000000000000000101"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0x0000000000000000000000000F00000000000000000000000000000000000000"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // external
                d("0xF000000000000000000000000000000000000000000000000000000000000000"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
        ];

        test_binary(SHR, &cases);
    }

    #[test]
    fn byte() {
        // TODO
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
        // TODO
    }

    #[test]
    fn gt() {
        let cases = [
            (
                // a=-4 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-4 b=-5 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=-5 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30000 b=-30000 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30000 b=-30001 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=-30001 b=-30000 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=1 b=1 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=2 b=1 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=1 b=2 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30000 b=30000 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30001 b=30000 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=30000 b=30001 r=0
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
            (
                // a=-4 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-4 b=-5 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-5 b=-4 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFB"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffFC"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=-30000 b=-30000 r=0
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30000 b=-30001 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=-30001 b=-30000 r=1
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8ACF"),
                d("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8AD0"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=1 b=1 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=2 b=1 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=1 b=2 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
                d("0x0000000000000000000000000000000000000000000000000000000000000002"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
            (
                // a=30000 b=30000 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30001 b=30000 r=1
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000000000"),
            ),
            (
                // a=30000 b=30001 r=0
                d("0x0000000000000000000000000000000000000000000000000000000000007530"),
                d("0x0000000000000000000000000000000000000000000000000000000000007531"),
                d("0x0000000000000000000000000000000000000000000000000000000000000001"),
            ),
        ];

        test_binary(LT, &cases);
    }

    fn test_binary(opcode: u8, cases: &[(Vec<u8>, Vec<u8>, Vec<u8>)]) {
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
                    chunk[i] = chunk[CHUNK_LEN - i - 1];
                    chunk[CHUNK_LEN - i - 1] = tmp;
                }
            }
            assert_eq!(&global_memory[0..32], res_expected);
        }
    }
}
