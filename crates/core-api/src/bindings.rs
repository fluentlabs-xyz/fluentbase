use alloc::vec::Vec;
use fluentbase_codec::define_codec_struct;
use fluentbase_core_macros::derive_helpers_and_structs;

derive_helpers_and_structs! {
    extern "C" {
        fn _evm_create(
            value32_offset: *const u8,
            code_offset: *const u8,
            code_len: u32,
            gas_limit: u32,
        ) -> *mut u8; // out_address20_offset

        fn _evm_create2(
            value32_offset: *const u8,
            salt32_offset: *const u8,
            code_offset: *const u8,
            code_len: u32,
            gas_limit: u32,
        ) -> *mut u8; // out_address20_offset

        fn _evm_call(
            callee_address20_offset: *const u8,
            value32_offset: *const u8,
            args_offset: *const u8,
            args_size: u32,
            gas_limit: u32,
        ) -> *mut u8; // ret_offset
    }
}

derive_helpers_and_structs! {
    extern "C" {
        fn _wasm_create(
            value32_offset: *const u8,
            code_offset: *const u8,
            code_len: u32,
            gas_limit: u32,
        ) -> *mut u8; // out_address20_offset

        fn _wasm_create2(
            value32_offset: *const u8,
            salt32_offset: *const u8,
            code_offset: *const u8,
            code_len: u32,
            gas_limit: u32,
        ) -> *mut u8; // out_address20_offset

        fn _wasm_call(
            callee_address20_offset: *const u8,
            value32_offset: *const u8,
            args_offset: *const u8,
            args_size: u32,
            gas_limit: u32,
        );
    }
}
