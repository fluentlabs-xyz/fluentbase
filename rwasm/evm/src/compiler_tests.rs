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

    fn test(evm_bytecode_bytes: &Vec<u8>) {
        let evm_binary = Bytes::from(evm_bytecode_bytes.clone());

        let mut compiler = EvmCompiler::new(evm_binary.as_ref(), false);

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);

        let mut preamble = InstructionSet::new();
        preamble.op_i32_const(100);
        preamble.op_memory_grow();
        preamble.op_drop();
        preamble.write_binary(&mut binary_format_writer).unwrap();

        compiler
            .instruction_set
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let rwasm_binary = &binary_format_writer.to_vec();

        let mut trace_binary = ReducedModule::new(&binary_format_writer.to_vec())
            .unwrap()
            .trace_binary();
        println!("\nrmodule.trace_binary(): \n{}\n", trace_binary);
        let mut rmodule = ReducedModule::new(rwasm_binary).unwrap();
        // let import_linker = ImportLinker::default();
        // let config = Config::default();
        // let engine = Engine::new(&config);
        // let mut module_builder =
        //     rmodule.to_module_builder(&engine, &import_linker, FuncType::new([], []));
        // module_builder.push_default_memory(100, None).unwrap();
        // let module = module_builder.finish();

        rmodule
            .bytecode()
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let result = Runtime::run(rwasm_binary, &Vec::new(), 0);
        assert!(result.is_ok());
        let execution_result = result.unwrap();
        debug!("\nlogs:");
        for log in execution_result.tracer().logs.iter() {
            if log.memory_changes.len() > 0 {
                debug!("memory_changes: {:?}", log.memory_changes)
            }
        }
        debug!(
            "\nexecution_result.tracer() (exit_code {}): \n{:#?}\n",
            execution_result.data().exit_code(),
            execution_result.tracer()
        );
    }

    #[test]
    fn simple() {
        let evm_bytecode_bytes: Vec<u8> = vec![
            PUSH0, PUSH1, 0x80, PUSH1, 0x40, PUSH8, 0, 0, 0, 0, 0, 0, 1, 246,
        ];

        test(&evm_bytecode_bytes);
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

        test(&evm_bytecode_bytes);
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

        test(&evm_bytecode_bytes);
    }

    #[test]
    fn lt_opcode() {
        let offset = 1;
        let a0 = 1;
        let b0 = 2;
        let evm_bytecode_bytes: Vec<u8> = vec![
            // args: `mem_offset` a=1 b=0
            // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
            PUSH1, offset, PUSH1, a0, PUSH1, b0, LT,
        ];

        test(&evm_bytecode_bytes);
    }

    #[test]
    fn gt_opcode() {
        let offset = 0;
        let a0_0 = 1;
        let a1_0 = 2;
        let b0_0 = 1;
        let b1_0 = 1;
        // if a > b
        let evm_bytecode_bytes: Vec<u8> = vec![
            // op: `mem_offset` a=1 b=0
            // TODO need evm preprocessing to automatically insert offset arg (PUSH1 0)
            PUSH1, offset, PUSH9, a1_0, 0, 0, 0, 0, 0, 0, 0, a0_0, PUSH9, b1_0, 0, 0, 0, 0, 0, 0, 0,
            b0_0, GT,
        ];

        test(&evm_bytecode_bytes);
    }
}
