mod syscalls {
    extern "C" {
        pub fn log(str: *const u8, len: u64);
    }
}

#[no_mangle]
pub fn entrypoint() -> u64 {
    let data = "Hi there!\n";
    unsafe {
        syscalls::log(data.as_ptr(), data.len() as u64);
    }
    0
}
