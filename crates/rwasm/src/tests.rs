use crate::{RwasmExecutor, SimpleCallContext, SimpleCallHandler};
use core::str::from_utf8;
use rwasm::rwasm::RwasmModule;

#[allow(unused)]
fn trace_rwasm(rwasm_bytecode: &[u8]) {
    let rwasm_module = RwasmModule::new(rwasm_bytecode).unwrap();
    let mut func_length = 0usize;
    let mut expected_func_length = rwasm_module
        .func_section
        .first()
        .copied()
        .unwrap_or(u32::MAX) as usize;
    let mut func_index = 0usize;
    println!("\n -- function #{} -- ", func_index);
    for (i, instr) in rwasm_module.code_section.instr.iter().enumerate() {
        println!("{:02}: {:?}", i, instr);
        func_length += 1;
        if func_length == expected_func_length {
            func_index += 1;
            expected_func_length = rwasm_module
                .func_section
                .get(func_index)
                .copied()
                .unwrap_or(u32::MAX) as usize;
            if expected_func_length != u32::MAX as usize {
                println!("\n -- function #{} -- ", func_index);
            }
            func_length = 0;
        }
    }
    println!("\n")
}

#[test]
fn test_execute_rwasm_bytecode() {
    let greeting_rwasm = include_bytes!("../../../examples/greeting/lib.rwasm");
    trace_rwasm(greeting_rwasm);
    let mut executor = RwasmExecutor::<SimpleCallHandler, SimpleCallContext>::parse(
        greeting_rwasm,
        None,
        SimpleCallContext::default(),
    )
    .unwrap();
    let exit_code = executor.run().unwrap();
    assert_eq!(exit_code, 0);
    let utf8_output = from_utf8(&executor.store.context.output).unwrap();
    assert_eq!(utf8_output, "Hello, World");
}
