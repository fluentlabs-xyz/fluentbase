// #![no_std]

#[cfg(feature = "runtime")]
mod runtime;
#[cfg(not(feature = "runtime"))]
mod rwasm;

#[cfg(feature = "runtime")]
pub use runtime::SDK;
#[cfg(not(feature = "runtime"))]
pub use rwasm::SDK;

pub trait CryptoPlatformSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon2(
        fa_offset: *const u8,
        fb_offset: *const u8,
        domain_offset: *const u8,
        output_offset: *mut u8,
    );
}

pub trait EccPlatformSDK {
    fn ecc_secp256k1_verify(digest: &[u8], sig: &[u8], pk_expected: &[u8], rec_id: u8) -> bool;
    fn ecc_secp256k1_recover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) -> bool;
}

pub trait MptPlatformSDK {}

pub trait RwasmPlatformSDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32;
    fn rwasm_transact(
        code_offset: i32,
        code_len: i32,
        input_offset: i32,
        input_len: i32,
        output_offset: i32,
        output_len: i32,
    ) -> i32;
}

pub trait SysPlatformSDK {
    fn sys_read_slice(target: &mut [u8], offset: u32) -> u32 {
        Self::sys_read(target.as_mut_ptr(), offset, target.len() as u32)
    }
    fn sys_read(target: *mut u8, offset: u32, len: u32) -> u32;
    fn sys_write_slice(value: &[u8]) {
        Self::sys_write(value.as_ptr(), value.len() as u32)
    }
    fn sys_write(offset: *const u8, len: u32);
    fn sys_halt(exit_code: i32);
}

pub trait ZktriePlatformSDK {}

// #[cfg(not(feature = "std"))]
// #[panic_handler]
// #[inline(always)]
// fn panic(info: &core::panic::PanicInfo) -> ! {
//     if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
//         sys_write(panic_message.as_ptr() as u32, panic_message.len() as u32);
//     }
//     sys_panic();
//     loop {}
// }

// #[cfg(not(feature = "std"))]
// #[lang = "eh_personality"]
// extern "C" fn eh_personality() {}
