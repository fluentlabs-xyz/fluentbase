use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    codec::{Encoder, FluentEncoder},
    Bytes,
    SharedContextInputV1,
    SysFuncIdx,
    PRECOMPILE_EVM,
    SYSCALL_ID_DELEGATE_CALL,
};
use rwasm::{
    instruction_set,
    rwasm::{BinaryFormat, RwasmModule},
};

pub const ENABLE_EVM_PROXY_CONTRACT: bool = false; //cfg!(feature = "evm-proxy");

pub fn create_rwasm_proxy_bytecode() -> Bytes {
    let mut memory_section = vec![0u8; 32 + 20];
    //  0..32: code hash
    // 32..52: precompile address
    memory_section[0..32].copy_from_slice(SYSCALL_ID_DELEGATE_CALL.as_slice()); // 32 bytes
    memory_section[32..52].copy_from_slice(PRECOMPILE_EVM.as_slice()); // 20 bytes
    debug_assert_eq!(memory_section.len(), 52);
    let code_section = instruction_set! {
        // alloc default memory
        I32Const(1) // number of pages (64kB memory in total)
        MemoryGrow // grow memory
        Drop // drop exit code (it can't fail here)
        // initialize memory segment
        I32Const(0) // destination
        I32Const(0) // source
        I32Const(memory_section.len() as u32) // length
        MemoryInit(0) // initialize 0 segment
        DataDrop(0) // mark 0 segment as dropped (required to satisfy WASM standards)
        // copy input (EVM bytecode can't exceed 2*24kB, so this op is safe)
        I32Const(52) // target
        I32Const(<SharedContextInputV1 as FluentEncoder>::HEADER_SIZE as u32) // offset
        Call(SysFuncIdx::INPUT_SIZE) // length=input_size-header_size
        I32Const(<SharedContextInputV1 as FluentEncoder>::HEADER_SIZE as u32)
        I32Sub
        Call(SysFuncIdx::READ)
        // delegate call
        I32Const(0) // hash32_ptr
        I32Const(32) // input_ptr
        Call(SysFuncIdx::INPUT_SIZE) // input_len=input_size-header_size+20
        I32Const(<SharedContextInputV1 as FluentEncoder>::HEADER_SIZE as u32)
        I32Sub
        I32Const(20)
        I32Add
        I32Const(0) // fuel_limit
        Call(SysFuncIdx::STATE) // state
        Call(SysFuncIdx::EXEC)
        // forward return data into output
        I32Const(0) // offset
        Call(SysFuncIdx::OUTPUT_SIZE) // length
        Call(SysFuncIdx::FORWARD_OUTPUT)
        // exit with the resulting exit code
        Call(SysFuncIdx::EXIT)
    };
    let func_section = vec![code_section.len() as u32];
    let evm_loader_module = RwasmModule {
        code_section,
        memory_section,
        func_section,
        ..Default::default()
    };
    let mut rwasm_bytecode = Vec::new();
    evm_loader_module
        .write_binary_to_vec(&mut rwasm_bytecode)
        .unwrap();
    rwasm_bytecode.into()
}
