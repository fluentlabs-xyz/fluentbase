#![cfg_attr(not(feature = "runtime"), no_std)]
#![feature(inherent_associated_types)]

extern crate alloc;
extern crate lol_alloc;

#[cfg(feature = "runtime")]
mod runtime;
#[cfg(not(feature = "runtime"))]
mod rwasm;

#[cfg(feature = "evm")]
pub mod evm;

pub struct SDK;

pub trait CryptoPlatformSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon(data: &[u8], output: &mut [u8]);
    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        domain_data: &[u8; 32],
        output: &mut [u8],
    );
}

pub trait EccPlatformSDK {
    fn ecc_secp256k1_verify(digest: &[u8], sig: &[u8], pk_expected: &[u8], rec_id: u8) -> bool;
    fn ecc_secp256k1_recover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) -> bool;
}

pub trait MptPlatformSDK {
    fn mpt_open();
    fn mpt_update(key: &[u8], value: &[u8]);
    fn mpt_get(key: &[u8], output: &mut [u8]) -> i32;
    fn mpt_root(output: &mut [u8]) -> i32;
}

pub trait RwasmPlatformSDK {
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32;
    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel_limit: u32,
    ) -> i32;
}

pub trait SysPlatformSDK {
    fn sys_read(target: &mut [u8], offset: u32) -> u32;
    fn sys_write(value: &[u8]);
    fn sys_halt(exit_code: i32);
    fn sys_state() -> u32;
}

pub trait ZktriePlatformSDK {
    fn zktrie_open();
    fn zktrie_update_nonce(key: &[u8], value: &[u8; 32]);
    fn zktrie_update_balance(key: &[u8], value: &[u8; 32]);
    fn zktrie_update_storage_root(key: &[u8], value: &[u8; 32]);
    fn zktrie_update_code_hash(key: &[u8], value: &[u8; 32]);
    fn zktrie_update_code_size(key: &[u8], value: &[u8; 32]);
    fn zktrie_get_nonce(key: &[u8]) -> [u8; 32];
    fn zktrie_get_balance(key: &[u8]) -> [u8; 32];
    fn zktrie_get_storage_root(key: &[u8]) -> [u8; 32];
    fn zktrie_get_code_hash(key: &[u8]) -> [u8; 32];
    fn zktrie_get_code_size(key: &[u8]) -> [u8; 32];
    fn zktrie_update_store(key: &[u8], value: &[u8; 32]);
    fn zktrie_get_store(key: &[u8]) -> [u8; 32];
}

// #[deprecated(note = "this SDK extension will be removed in the future, use on your own risk")]
#[cfg(feature = "evm")]
pub trait EvmPlatformSDK {
    fn evm_sload(key: &[u8], value: &mut [u8]);
    fn evm_sstore(key: &[u8], value: &[u8]);
}

#[cfg(not(feature = "runtime"))]
#[panic_handler]
#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(panic_message) = info.payload().downcast_ref::<&str>() {
        SDK::sys_write(panic_message.as_bytes());
    }
    SDK::sys_halt(-71);
    loop {}
}

#[cfg(not(feature = "runtime"))]
#[global_allocator]
#[cfg(target_arch = "wasm32")]
static ALLOCATOR: lol_alloc::AssumeSingleThreaded<lol_alloc::LeakingAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::LeakingAllocator::new()) };
