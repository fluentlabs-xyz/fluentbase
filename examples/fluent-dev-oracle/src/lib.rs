#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI, ContextReader};
use alloy_primitives::U256;

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input = sdk.bytes_input();
    
    // تأكد من أن المدخلات كافية لتحويلها لـ U256 (32 بايت)
    if input.len() >= 32 {
        let caller_address = sdk.context().contract_caller();
        
        // تحويل أول 32 بايت من المدخلات لمفتاح التخزين
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&input[0..32]);
        let storage_key = U256::from_be_bytes(key_bytes);
        
        // تحويل عنوان المطور لقيمة التخزين
        let mut val_bytes = [0u8; 32];
        val_bytes[12..32].copy_from_slice(caller_address.as_slice());
        let storage_value = U256::from_be_bytes(val_bytes);
        
        // التخزين في البلوكشين
        sdk.write_storage(storage_key, storage_value);
        
        // تسجيل الأثر (Log) لترك بصمة FreeDropOracle
        sdk.write(b"Dev Registered via FreeDropOracle");
    }
}

entrypoint!(main_entry);