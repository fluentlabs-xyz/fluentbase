use crate::{
    alloc_slice,
    byteorder::{ByteOrder, LittleEndian},
    Address, BytecodeOrHash, Bytes, InterruptAPI, B256, STATE_MAIN, U256,
};
use alloc::vec;

pub const SYSCALL_ID_STORAGE_READ: B256 = B256::with_last_byte(0x01);
pub const SYSCALL_ID_STORAGE_WRITE: B256 = B256::with_last_byte(0x02);
pub const SYSCALL_ID_CALL: B256 = B256::with_last_byte(0x03);
pub const SYSCALL_ID_STATIC_CALL: B256 = B256::with_last_byte(0x04);
pub const SYSCALL_ID_CALL_CODE: B256 = B256::with_last_byte(0x05);
pub const SYSCALL_ID_DELEGATE_CALL: B256 = B256::with_last_byte(0x06);
pub const SYSCALL_ID_CREATE: B256 = B256::with_last_byte(0x07);
pub const SYSCALL_ID_CREATE2: B256 = B256::with_last_byte(0x08);
pub const SYSCALL_ID_EMIT_LOG: B256 = B256::with_last_byte(0x09);
pub const SYSCALL_ID_DESTROY_ACCOUNT: B256 = B256::with_last_byte(0x0a);
pub const SYSCALL_ID_BALANCE: B256 = B256::with_last_byte(0x0b);
pub const SYSCALL_ID_SELF_BALANCE: B256 = B256::with_last_byte(0x0c);
pub const SYSCALL_ID_CODE_SIZE: B256 = B256::with_last_byte(0x0d);
pub const SYSCALL_ID_CODE_HASH: B256 = B256::with_last_byte(0x0e);
pub const SYSCALL_ID_CODE_COPY: B256 = B256::with_last_byte(0x0f);
pub const SYSCALL_ID_TRANSIENT_READ: B256 = B256::with_last_byte(0x11);
pub const SYSCALL_ID_TRANSIENT_WRITE: B256 = B256::with_last_byte(0x12);
pub const SYSCALL_ID_BLOCK_HASH: B256 = B256::with_last_byte(0x13);

pub const SYSCALL_ID_METADATA_WRITE: B256 = B256::with_last_byte(0x40);
pub const SYSCALL_ID_METADATA_SIZE: B256 = B256::with_last_byte(0x41);
pub const SYSCALL_ID_METADATA_CREATE: B256 = B256::with_last_byte(0x42);
pub const SYSCALL_ID_METADATA_COPY: B256 = B256::with_last_byte(0x43);

pub const SYSCALL_ID_METADATA_STORAGE_READ: B256 = B256::with_last_byte(0x44);
pub const SYSCALL_ID_METADATA_STORAGE_WRITE: B256 = B256::with_last_byte(0x45);
pub const SYSCALL_ID_METADATA_ACCOUNT_OWNER: B256 = B256::with_last_byte(0x46);

pub trait SyscallInterruptExecutor {
    fn write_storage(&mut self, slot: U256, value: U256) -> (u64, i64, i32);
    fn storage(&self, slot: &U256) -> (u64, i64, i32);
    fn metadata_write(
        &mut self,
        address: &Address,
        offset: u32,
        metadata: Bytes,
    ) -> (u64, i64, i32);
    fn metadata_size(&self, address: &Address) -> (u64, i64, i32);
    fn metadata_account_owner(&self, address: &Address) -> (u64, i64, i32);
    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> (u64, i64, i32);
    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> (u64, i64, i32);
    fn metadata_storage_read(&self, slot: &U256) -> (u64, i64, i32);
    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> (u64, i64, i32);
    fn write_transient_storage(&mut self, slot: U256, value: U256) -> (u64, i64, i32);
    fn transient_storage(&self, slot: &U256) -> (u64, i64, i32);
    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> (u64, i64, i32);
    fn self_balance(&self) -> (u64, i64, i32);
    fn balance(&self, address: &Address) -> (u64, i64, i32);
    fn block_hash(&self, block_number: u64) -> (u64, i64, i32);
    fn code_size(&self, address: &Address) -> (u64, i64, i32);
    fn code_hash(&self, address: &Address) -> (u64, i64, i32);
    fn code_copy(&self, address: &Address, code_offset: u64, code_length: u64) -> (u64, i64, i32);
    fn create(&mut self, salt: Option<U256>, value: &U256, init_code: &[u8]) -> (u64, i64, i32);
    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32);
    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32);
    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32);
    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32);
    fn destroy_account(&mut self, address: Address) -> (u64, i64, i32);
}

impl<T: InterruptAPI + ?Sized> SyscallInterruptExecutor for T {
    fn write_storage(&mut self, slot: U256, value: U256) -> (u64, i64, i32) {
        let mut input = [0u8; U256::BYTES + U256::BYTES];
        unsafe {
            core::ptr::copy(
                slot.as_limbs().as_ptr() as *mut u8,
                input.as_mut_ptr(),
                U256::BYTES,
            );
            core::ptr::copy(
                value.as_limbs().as_ptr() as *mut u8,
                input.as_mut_ptr().add(U256::BYTES),
                U256::BYTES,
            );
        }
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_STORAGE_WRITE),
            &input,
            None,
            STATE_MAIN,
        )
    }
    fn storage(&self, slot: &U256) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_STORAGE_READ),
            slot.as_le_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn metadata_write(
        &mut self,
        address: &Address,
        offset: u32,
        metadata: Bytes,
    ) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 20 + 4 + metadata.len()];
        buffer[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u32(&mut buffer[20..24], offset);
        buffer[24..].copy_from_slice(&metadata);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_WRITE),
            &buffer,
            None,
            STATE_MAIN,
        )
    }
    fn metadata_size(&self, address: &Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_SIZE),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn metadata_account_owner(&self, address: &Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_ACCOUNT_OWNER),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; U256::BYTES + metadata.len()];
        buffer[0..32].copy_from_slice(salt.to_be_bytes::<32>().as_slice());
        buffer[32..].copy_from_slice(&metadata);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_CREATE),
            &buffer,
            None,
            STATE_MAIN,
        )
    }
    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> (u64, i64, i32) {
        let mut buffer = [0u8; 20 + 4 + 4];
        buffer[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u32(&mut buffer[20..24], offset);
        LittleEndian::write_u32(&mut buffer[24..28], length);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_COPY),
            &buffer,
            None,
            STATE_MAIN,
        )
    }
    fn metadata_storage_read(&self, slot: &U256) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_STORAGE_READ),
            &slot.to_le_bytes::<{ U256::BYTES }>(),
            None,
            STATE_MAIN,
        )
    }
    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> (u64, i64, i32) {
        let mut input = [0u8; U256::BYTES * 2];
        input[..U256::BYTES].copy_from_slice(slot.as_le_slice());
        input[U256::BYTES..].copy_from_slice(&value.to_le_bytes::<{ U256::BYTES }>());
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_METADATA_STORAGE_WRITE),
            &input,
            None,
            STATE_MAIN,
        )
    }
    fn write_transient_storage(&mut self, slot: U256, value: U256) -> (u64, i64, i32) {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_TRANSIENT_WRITE),
            &input,
            None,
            STATE_MAIN,
        )
    }
    fn transient_storage(&self, slot: &U256) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_TRANSIENT_READ),
            slot.as_le_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 1 + topics.len() * B256::len_bytes()];
        assert!(topics.len() <= 4);
        buffer[0] = topics.len() as u8;
        for (i, topic) in topics.iter().enumerate() {
            buffer[(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())]
                .copy_from_slice(topic.as_slice());
        }
        buffer.extend_from_slice(data);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_EMIT_LOG),
            &buffer,
            None,
            STATE_MAIN,
        )
    }
    fn self_balance(&self) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_SELF_BALANCE),
            &[],
            None,
            STATE_MAIN,
        )
    }
    fn balance(&self, address: &Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_BALANCE),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn block_hash(&self, block_number: u64) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_BLOCK_HASH),
            &block_number.to_le_bytes(),
            None,
            STATE_MAIN,
        )
    }
    fn code_size(&self, address: &Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_SIZE),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn code_hash(&self, address: &Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_HASH),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
    fn code_copy(&self, address: &Address, code_offset: u64, code_length: u64) -> (u64, i64, i32) {
        let mut input = [0u8; 20 + 8 * 2];
        input[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u64(&mut input[20..28], code_offset);
        LittleEndian::write_u64(&mut input[28..36], code_length);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_COPY),
            &input,
            None,
            STATE_MAIN,
        )
    }
    fn create(&mut self, salt: Option<U256>, value: &U256, init_code: &[u8]) -> (u64, i64, i32) {
        if let Some(salt) = salt {
            let buffer = alloc_slice(32 + 32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..64].copy_from_slice(salt.as_le_slice());
            buffer[64..].copy_from_slice(init_code);
            self.interrupt(
                BytecodeOrHash::Hash(SYSCALL_ID_CREATE2),
                &buffer,
                None,
                STATE_MAIN,
            )
        } else {
            let buffer = alloc_slice(32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..].copy_from_slice(init_code);
            self.interrupt(
                BytecodeOrHash::Hash(SYSCALL_ID_CREATE),
                &buffer,
                None,
                STATE_MAIN,
            )
        }
    }
    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_CALL),
            &buffer,
            fuel_limit,
            STATE_MAIN,
        )
    }
    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_CALL_CODE),
            &buffer,
            fuel_limit,
            STATE_MAIN,
        )
    }
    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_DELEGATE_CALL),
            &buffer,
            fuel_limit,
            STATE_MAIN,
        )
    }
    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> (u64, i64, i32) {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_STATIC_CALL),
            &buffer,
            fuel_limit,
            STATE_MAIN,
        )
    }
    fn destroy_account(&mut self, address: Address) -> (u64, i64, i32) {
        self.interrupt(
            BytecodeOrHash::Hash(SYSCALL_ID_DESTROY_ACCOUNT),
            address.as_slice(),
            None,
            STATE_MAIN,
        )
    }
}
