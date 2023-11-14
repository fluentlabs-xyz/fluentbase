#[cfg(test)]
mod evm_to_rwasm_tests {
    use alloy_primitives::Bytes;

    use fluentbase_rwasm::rwasm::{BinaryFormat, BinaryFormatWriter, ReducedModule};

    use crate::compiler::EvmCompiler;
    use crate::interpreter::instruction_result::InstructionResult;

    #[test]
    fn simple_bytecode_test() {
        // 0x01: ADD
        // 0x50  POP
        // 0x5F: PUSH0
        // 0x60: PUSH1
        // 0x61: PUSH1
        // 0x62: PUSH2
        // 0x63: PUSH3
        // 0x64: PUSH4
        // 0x65: PUSH5
        // 0x66: PUSH6
        // 0x67: PUSH7
        // 0x68: PUSH8

        // let evm_bytecode_hex = "0x5F60806040";
        let evm_bytecode_bytes: &[u8] =
            &[0x5F, 0x60, 0x80, 0x60, 0x40, 0x68, 1, 2, 3, 4, 5, 6, 7, 8];
        // let evm_bytecode_bytes = hex::decode(evm_bytecode_hex).unwrap();
        let evm_bytecode_bytes = Bytes::from(evm_bytecode_bytes);
        println!("evm_bytecode_bytes: {:x?}", evm_bytecode_bytes.as_ref());

        let mut compiler = EvmCompiler::new(evm_bytecode_bytes.as_ref());

        let res = compiler.translate();
        assert_eq!(res, InstructionResult::Stop);

        let mut buffer = vec![0; 1024 * 1024];
        let mut buffer_writer = BinaryFormatWriter::new(&mut buffer);
        let rwasm_bytecode_bytes_len = compiler
            .instruction_set
            .write_binary(&mut buffer_writer)
            .unwrap();
        println!("wasm_bytecode_bytes_len: {}", rwasm_bytecode_bytes_len);
        // println!("buffer bytes {:?}", buffer);
        // println!("buffer hex {:x?}", buffer);
        let rwasm_bytecode_bytes = &buffer[0..rwasm_bytecode_bytes_len];
        println!("rwasm bytecode bytes: {:?}", rwasm_bytecode_bytes);

        // lets try to execute and fetch result?
        let mut rmodule = ReducedModule::new(rwasm_bytecode_bytes).unwrap();
        let rwasm_binary_trace = rmodule.trace_binary();
        println!("rwasm_binary_trace: {:?}", &rwasm_binary_trace);

        // assert!(false)
    }
}
