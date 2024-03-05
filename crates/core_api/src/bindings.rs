use fluentbase_types::ExitCode;

extern "C" {
    fn _create(
        value32_offset: *const u8,
        code_offset: *const u8,
        code_len: u32,
        gas_limit: u32,
        out_address20_offset: *mut u8,
    ) -> ExitCode;

    fn _create2(
        value32_offset: *const u8,
        salt32_offset: *const u8,
        code_offset: *const u8,
        code_len: u32,
        gas_limit: u32,
        out_address20_offset: *mut u8,
    ) -> ExitCode;

    fn _call(
        callee_address20_offset: *const u8,
        value32_offset: *const u8,
        args_offset: *const u8,
        args_size: u32,
        ret_offset: *mut u8,
        ret_size: u32,
        gas_limit: u32,
    ) -> ExitCode;
}
