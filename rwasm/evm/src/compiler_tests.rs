#[cfg(test)]
mod evm_to_rwasm_tests {
    use crate::{
        compiler::EvmCompiler,
        translator::{
            instruction_result::InstructionResult,
            instructions::opcode::{EQ, PUSH0, PUSH1, PUSH4, PUSH8},
        },
    };
    use alloy_primitives::Bytes;
    use fluentbase_runtime::Runtime;
    use fluentbase_rwasm::{
        rwasm::{BinaryFormat, BinaryFormatWriter, ImportLinker, InstructionSet, ReducedModule},
        Config,
        Engine,
        FuncType,
    };
    use log::debug;

    #[test]
    fn simple_bytecode() {
        let evm_bytecode_bytes: &[u8] = &[
            PUSH0, PUSH1, 0x80, PUSH1, 0x40, PUSH8, 0, 0, 0, 0, 0, 0, 1, 246,
        ];
        let evm_bytecode_bytes = Bytes::from(evm_bytecode_bytes);
        debug!("evm_bytecode_bytes: {:x?}", evm_bytecode_bytes.as_ref());

        let mut compiler = EvmCompiler::new(evm_bytecode_bytes.as_ref(), false);

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut buffer_writer = BinaryFormatWriter::new(&mut buffer);
        let rwasm_bytecode_bytes_len = compiler
            .instruction_set
            .write_binary(&mut buffer_writer)
            .unwrap();
        debug!("rwasm_bytecode_bytes_len: {}", rwasm_bytecode_bytes_len);
        let rwasm_bytecode_bytes = &buffer[0..rwasm_bytecode_bytes_len];
        debug!("rwasm bytecode bytes: {:?}", rwasm_bytecode_bytes);

        let mut rmodule = ReducedModule::new(rwasm_bytecode_bytes).unwrap();
        let rwasm_binary_trace = rmodule.trace_binary();
        debug!("rwasm_binary_trace: {:?}", &rwasm_binary_trace);
    }

    #[test]
    fn eq_opcode() {
        let evm_bytecode_bytes: &[u8] = &[
            // op: EQ mem_offset a b
            //  PUSH8, 0, 0, 0, 0, 0, 0, 0, 0, PUSH8, 0, 0, 0, 0, 0, 0, 0, 0, PUSH4, 0, 0, 0, 0,
            EQ,
        ];
        let evm_binary = Bytes::from(evm_bytecode_bytes);
        debug!(
            "evm_binary (len {}) {:x?}",
            evm_binary.len(),
            evm_binary.as_ref(),
        );

        let mut compiler = EvmCompiler::new(evm_binary.as_ref(), false);

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);
        debug!(
            "compiler.instruction_set.trace_binary(): {:?}",
            compiler.instruction_set.trace_binary()
        );

        let mut buffer = vec![0; 1024 * 1024];
        let mut binary_format_writer = BinaryFormatWriter::new(&mut buffer);

        let mut preamble = InstructionSet::new();
        preamble.op_i32_const(100);
        preamble.op_memory_grow();
        preamble.op_drop();
        preamble.write_binary(&mut binary_format_writer).unwrap();

        // let mut postamble = InstructionSet::new();
        // postamble.op_i32_const(15);
        // postamble.op_br(3);
        // compiler.instruction_set.extend(postamble);

        compiler
            .instruction_set
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let rwasm_binary = &binary_format_writer.to_vec();

        debug!(
            "rmodule.trace_binary(): {:?}",
            ReducedModule::new(&binary_format_writer.to_vec())
                .unwrap()
                .trace_binary()
        );
        let mut rmodule = ReducedModule::new(rwasm_binary).unwrap();
        let mut import_linker = ImportLinker::default();
        let config = Config::default();
        let engine = Engine::new(&config);
        let mut module_builder =
            rmodule.to_module_builder(&engine, &import_linker, FuncType::new([], []));
        module_builder.push_default_memory(100, None).unwrap();
        let module = module_builder.finish();
        debug!("module.exports:");
        for export in module.exports() {
            debug!("export {} (idx {:?})", export.name(), export.index());
        }

        rmodule
            .bytecode()
            .write_binary(&mut binary_format_writer)
            .unwrap();
        let result = Runtime::run(rwasm_binary, &Vec::new(), 0);
        assert!(result.is_ok());
        let execution_result = result.unwrap();
        println!(
            "execution_result (exit_code {}): {:?}",
            execution_result.data().exit_code(),
            execution_result
        );
    }
}
