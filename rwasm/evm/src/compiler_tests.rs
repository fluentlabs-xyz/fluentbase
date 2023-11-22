#[cfg(test)]
mod evm_to_rwasm_tests {
    use alloy_primitives::Bytes;
    use fluentbase_runtime::Runtime;
    use log::debug;

    use fluentbase_rwasm::rwasm::{
        BinaryFormat, BinaryFormatWriter, InstructionSet, ReducedModule,
    };

    use crate::compiler::EvmCompiler;
    use crate::translator::instruction_result::InstructionResult;
    use crate::translator::instructions::opcode::{EQ, PUSH0, PUSH1, PUSH4, PUSH8};

    #[test]
    fn simple_bytecode() {
        let evm_bytecode_bytes: &[u8] = &[
            PUSH0, PUSH1, 0x80, PUSH1, 0x40, PUSH8, 0, 0, 0, 0, 0, 0, 1, 246,
        ];
        let evm_bytecode_bytes = Bytes::from(evm_bytecode_bytes);
        debug!("evm_bytecode_bytes: {:x?}", evm_bytecode_bytes.as_ref());

        let mut compiler = EvmCompiler::new(evm_bytecode_bytes.as_ref());

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
            // EQ b a mem_offset
            PUSH8, 0, 0, 0, 0, 0, 0, 0, 0, PUSH8, 0, 0, 0, 0, 0, 0, 0, 0, PUSH4, 0, 0, 0, 0, EQ,
        ];
        let evm_binary = Bytes::from(evm_bytecode_bytes);
        debug!(
            "evm_binary (len {}) {:x?}",
            evm_binary.len(),
            evm_binary.as_ref(),
        );

        let mut compiler = EvmCompiler::new(evm_binary.as_ref());

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut buffer_writer = BinaryFormatWriter::new(&mut buffer);

        let mut preambule = InstructionSet::new();
        preambule.op_i32_const(100);
        preambule.op_memory_grow();
        preambule.op_drop();
        preambule.write_binary(&mut buffer_writer).unwrap();

        compiler
            .instruction_set
            .write_binary(&mut buffer_writer)
            .unwrap();
        let rwasm_binary = &buffer_writer.to_vec();
        debug!(
            "rwasm_binary (len {}): {:?}",
            rwasm_binary.len(),
            rwasm_binary
        );

        let mut rmodule = ReducedModule::new(rwasm_binary).unwrap();
        debug!("rmodule.trace_binary(): {:?}", rmodule.trace_binary());
        let exec_result = Runtime::run(rwasm_binary, &Vec::new(), 10_000_000);
        assert!(exec_result.is_ok());
        let exec_result = exec_result.unwrap();
        let tracer = exec_result.tracer();
        let runtime_context = exec_result.data();
        debug!("runtime_context.output(): {:x?}", runtime_context.output());
        debug!("tracer.global_memory: {:x?}", tracer.global_memory);
    }
}
