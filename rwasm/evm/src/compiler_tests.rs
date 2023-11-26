#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{EQ, GT, LT, PUSH0, PUSH1, PUSH2, PUSH5, PUSH8, PUSH9, SHL},
        },
    };
    use alloy_primitives::Bytes;
    use fluentbase_runtime::Runtime;
    use fluentbase_rwasm::rwasm::{
        BinaryFormat,
        BinaryFormatWriter,
        InstructionSet,
        ReducedModule,
    };
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
        let global_memory = global_memory.to_vec();
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
    fn simple() {
        let evm_bytecode_bytes: Vec<u8> = vec![
            PUSH0, PUSH1, 0x80, PUSH1, 0x40, PUSH8, 0, 0, 0, 0, 0, 0, 1, 246,
        ];

        run_test(&evm_bytecode_bytes);
    }

    #[test]
    fn eq_opcode() {
        let offset = 1;
        let a0 = 1;
        let b0 = 2;
        let evm_bytecode_bytes: Vec<u8> = vec![
            // args: `mem_offset` a=1 b=0
            // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
            PUSH1, offset, PUSH1, a0, PUSH1, b0, EQ,
        ];

        run_test(&evm_bytecode_bytes);
    }

    #[test]
    fn shl_opcode() {
        let offset = 1;
        let a0 = 1;
        let b0 = 2;
        let evm_bytecode_bytes: Vec<u8> = vec![
            // args: `mem_offset` a=1 b=0
            // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
            PUSH1, offset, PUSH1, a0, PUSH1, b0, SHL,
        ];

        run_test(&evm_bytecode_bytes);
    }

    #[test]
    fn lt_opcode() {
        let offset = 0;
        let a0_0 = 1;
        let a1_0 = 2;
        let b0_0 = 2;
        let b1_0 = 1;
        // if a > b
        let evm_bytecode_bytes: Vec<u8> = vec![
            // op: `mem_offset` a=1 b=0
            // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
            PUSH1, offset, PUSH9, a1_0, 0, 0, 0, 0, 0, 0, 0, a0_0, PUSH9, b1_0, 0, 0, 0, 0, 0, 0, 0,
            b0_0, LT,
        ];

        run_test(&evm_bytecode_bytes);
    }

    #[test]
    fn gt_opcode() {
        let offset = 0;
        // BE repr of A and B params and expected RESULT
        let test_cases = [
            ((0, 1), (0, 1), 0u8),
            ((0, 2), (0, 1), 1u8),
            ((2, 1), (1, 1), 1u8),
            ((1, 1), (1, 1), 0),
        ];
        for case in &test_cases {
            let a1_0 = case.0 .0;
            let a0_0 = case.0 .1;
            let b1_0 = case.1 .0;
            let b0_0 = case.1 .1;
            let res_expected = case.2;
            // if a > b
            let evm_bytecode_bytes: Vec<u8> = vec![
                // op: `mem_offset` a=1 b=0
                // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
                PUSH1, offset, PUSH9, a1_0, 0, 0, 0, 0, 0, 0, 0, a0_0, PUSH9, b1_0, 0, 0, 0, 0, 0,
                0, 0, b0_0, GT,
            ];

            let global_memory = run_test(&evm_bytecode_bytes);
            let res = global_memory[32 - 8];
            assert_eq!(res, res_expected);
        }
    }
}
