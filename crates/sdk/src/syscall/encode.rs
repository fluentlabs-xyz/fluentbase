//! Pure syscall input encoding helpers (no `self`, no locks).
//!
//! Design goals:
//! - `*_size_hint(...) -> usize` for predictable allocation.
//! - `*_into(&mut impl BufMut, ...)` for zero-copy-ish building into caller buffer.
//! - Convenience `*_vec(...) -> Vec<u8>` helpers for easy call sites.
//!
//! Works with `Vec<u8>`, `bytes::BytesMut`, `smallvec::SmallVec`, etc.

use crate::{Address, B256, U256};
use fluentbase_types::bytes::BufMut;
// -------------------------
// Storage
// -------------------------

#[inline(always)]
pub const fn storage_read_size_hint() -> usize {
    U256::BYTES
}

#[inline(always)]
pub fn storage_read_into<B: BufMut>(out: &mut B, slot: &U256) {
    out.put_slice(slot.as_le_slice());
}

#[inline(always)]
pub const fn storage_write_size_hint() -> usize {
    U256::BYTES + U256::BYTES
}

#[inline(always)]
pub fn storage_write_into<B: BufMut>(out: &mut B, slot: &U256, value: &U256) {
    out.put_slice(slot.as_le_slice());
    out.put_slice(value.as_le_slice());
}

// -------------------------
// Metadata
// -------------------------

#[inline(always)]
pub const fn metadata_size_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn metadata_size_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

#[inline(always)]
pub const fn metadata_account_owner_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn metadata_account_owner_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

#[inline(always)]
pub const fn metadata_copy_size_hint() -> usize {
    Address::len_bytes() + size_of::<u32>() + size_of::<u32>()
}

#[inline(always)]
pub fn metadata_copy_into<B: BufMut>(out: &mut B, address: &Address, offset: u32, length: u32) {
    out.put_slice(address.as_slice());
    out.put_u32_le(offset);
    out.put_u32_le(length);
}

#[inline(always)]
pub const fn metadata_storage_read_size_hint() -> usize {
    U256::BYTES
}

#[inline(always)]
pub fn metadata_storage_read_into<B: BufMut>(out: &mut B, slot: &U256) {
    out.put_slice(slot.as_le_slice());
}

#[inline(always)]
pub fn metadata_storage_write_size_hint() -> usize {
    U256::BYTES + U256::BYTES
}

#[inline(always)]
pub fn metadata_storage_write_into<B: BufMut>(out: &mut B, slot: &U256, value: &U256) {
    out.put_slice(slot.as_le_slice());
    out.put_slice(value.as_le_slice());
}

#[inline(always)]
pub const fn metadata_write_size_hint(metadata_len: usize) -> usize {
    Address::len_bytes() + size_of::<u32>() + metadata_len
}

#[inline(always)]
pub fn metadata_write_into<B: BufMut>(
    out: &mut B,
    address: &Address,
    offset: u32,
    metadata: impl AsRef<[u8]>,
) {
    out.put_slice(address.as_slice());
    out.put_u32_le(offset);
    out.put_slice(metadata.as_ref());
}

#[inline(always)]
pub const fn metadata_create_size_hint(metadata_len: usize) -> usize {
    U256::BYTES + metadata_len
}

#[inline(always)]
pub fn metadata_create_into<B: BufMut>(out: &mut B, salt: &U256, metadata: &[u8]) {
    // NOTE: your original code used BE bytes for salt here; keep the exact standard.
    out.put_slice(&salt.to_be_bytes::<{ U256::BYTES }>());
    out.put_slice(metadata);
}

// -------------------------
// Transient storage
// -------------------------

#[inline(always)]
pub const fn transient_write_size_hint() -> usize {
    U256::BYTES + U256::BYTES
}

#[inline(always)]
pub fn transient_write_into<B: BufMut>(out: &mut B, slot: &U256, value: &U256) {
    // Preserve your “zero optimizes to zeros” semantics.
    let mut buf = [0u8; 64];
    if !slot.is_zero() {
        buf[0..32].copy_from_slice(slot.as_le_slice());
    }
    if !value.is_zero() {
        buf[32..64].copy_from_slice(value.as_le_slice());
    }
    out.put_slice(&buf);
}

#[inline(always)]
pub const fn transient_read_size_hint() -> usize {
    U256::BYTES
}

#[inline(always)]
pub fn transient_read_into<B: BufMut>(out: &mut B, slot: &U256) {
    out.put_slice(slot.as_le_slice());
}

// -------------------------
// Logs
// -------------------------

#[inline(always)]
pub fn emit_log_size_hint(topics_len: usize, data_len: usize) -> usize {
    // 1 byte topic count + topics * 32 + data
    1 + topics_len * B256::len_bytes() + data_len
}

#[inline(always)]
pub fn emit_log_into<B: BufMut>(out: &mut B, topics: &[B256], data: &[u8]) {
    debug_assert!(topics.len() <= 4);
    out.put_u8(topics.len() as u8);
    for t in topics {
        out.put_slice(t.as_slice());
    }
    out.put_slice(data);
}

// -------------------------
// Balance / block / code
// -------------------------

#[inline(always)]
pub const fn self_balance_size_hint() -> usize {
    0
}

#[inline(always)]
pub fn self_balance_into<B: BufMut>(_out: &mut B) {}

#[inline(always)]
pub const fn balance_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn balance_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

#[inline(always)]
pub const fn block_hash_size_hint() -> usize {
    8
}

#[inline(always)]
pub fn block_hash_into<B: BufMut>(out: &mut B, block_number: u64) {
    out.put_u64_le(block_number);
}

#[inline(always)]
pub const fn code_size_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn code_size_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

#[inline(always)]
pub const fn code_hash_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn code_hash_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

#[inline(always)]
pub const fn code_copy_size_hint() -> usize {
    Address::len_bytes() + size_of::<u64>() + size_of::<u64>()
}

#[inline(always)]
pub fn code_copy_into<B: BufMut>(
    out: &mut B,
    address: &Address,
    code_offset: u64,
    code_length: u64,
) {
    out.put_slice(address.as_slice());
    out.put_u64_le(code_offset);
    out.put_u64_le(code_length);
}

// -------------------------
// Create / Call family
// -------------------------

#[inline(always)]
pub fn create_size_hint(init_code_len: usize, has_salt: bool) -> usize {
    if has_salt {
        U256::BYTES + U256::BYTES + init_code_len
    } else {
        U256::BYTES + init_code_len
    }
}

#[inline(always)]
pub fn create_into<B: BufMut>(out: &mut B, salt: Option<&U256>, value: &U256, init_code: &[u8]) {
    out.put_slice(value.as_le_slice());
    if let Some(s) = salt {
        out.put_slice(s.as_le_slice());
    }
    out.put_slice(init_code);
}

#[inline(always)]
pub fn call_size_hint(input_len: usize, has_value: bool) -> usize {
    if has_value {
        Address::len_bytes() + U256::BYTES + input_len
    } else {
        Address::len_bytes() + input_len
    }
}

#[inline(always)]
pub fn call_into<B: BufMut>(out: &mut B, address: Address, value: Option<U256>, input: &[u8]) {
    out.put_slice(address.as_slice());
    if let Some(value) = value {
        out.put_slice(value.as_le_slice());
    }
    out.put_slice(input);
}

#[inline(always)]
pub fn delegate_call_size_hint(input_len: usize) -> usize {
    Address::len_bytes() + input_len
}

#[inline(always)]
pub fn delegate_call_into<B: BufMut>(out: &mut B, address: &Address, input: &[u8]) {
    out.put_slice(address.as_slice());
    out.put_slice(input);
}

#[inline(always)]
pub fn static_call_size_hint(input_len: usize) -> usize {
    Address::len_bytes() + input_len
}

#[inline(always)]
pub fn static_call_into<B: BufMut>(out: &mut B, address: &Address, input: &[u8]) {
    out.put_slice(address.as_slice());
    out.put_slice(input);
}

// -------------------------
// Account destruction
// -------------------------

#[inline(always)]
pub const fn destroy_account_size_hint() -> usize {
    Address::len_bytes()
}

#[inline(always)]
pub fn destroy_account_into<B: BufMut>(out: &mut B, address: &Address) {
    out.put_slice(address.as_slice());
}

// -------------------------
// Runtime upgrade
// -------------------------

#[inline(always)]
pub const fn upgrade_runtime_size_hint(wasm_bytecode_len: usize) -> usize {
    Address::len_bytes() + wasm_bytecode_len
}

#[inline(always)]
pub fn upgrade_runtime_into<B: BufMut>(out: &mut B, address: &Address, wasm_bytecode: &[u8]) {
    out.put_slice(address.as_slice());
    out.put_slice(wasm_bytecode);
}
