use crate::{
    alloc_slice,
    NativeAPI,
    GAS_LIMIT_SYSCALL_BALANCE,
    GAS_LIMIT_SYSCALL_DESTROY_ACCOUNT,
    GAS_LIMIT_SYSCALL_EMIT_LOG,
    GAS_LIMIT_SYSCALL_EXT_STORAGE_READ,
    GAS_LIMIT_SYSCALL_PREIMAGE_SIZE,
    GAS_LIMIT_SYSCALL_STORAGE_READ,
    GAS_LIMIT_SYSCALL_STORAGE_WRITE,
    GAS_LIMIT_SYSCALL_TRANSIENT_READ,
    GAS_LIMIT_SYSCALL_TRANSIENT_WRITE,
    STATE_MAIN,
    SYSCALL_ID_BALANCE,
    SYSCALL_ID_CALL,
    SYSCALL_ID_CALL_CODE,
    SYSCALL_ID_CREATE,
    SYSCALL_ID_CREATE2,
    SYSCALL_ID_DELEGATE_CALL,
    SYSCALL_ID_DESTROY_ACCOUNT,
    SYSCALL_ID_EMIT_LOG,
    SYSCALL_ID_EXT_STORAGE_READ,
    SYSCALL_ID_PREIMAGE_COPY,
    SYSCALL_ID_PREIMAGE_SIZE,
    SYSCALL_ID_STATIC_CALL,
    SYSCALL_ID_STORAGE_READ,
    SYSCALL_ID_STORAGE_WRITE,
    SYSCALL_ID_TRANSIENT_READ,
    SYSCALL_ID_TRANSIENT_WRITE,
    SYSCALL_ID_WRITE_PREIMAGE,
    U256,
};
use alloc::vec;
use alloy_primitives::{Address, Bytes, B256};
use byteorder::{ByteOrder, LittleEndian};

pub trait SyscallAPI {
    fn syscall_storage_read(&self, slot: &U256) -> U256;
    fn syscall_storage_write(&self, slot: &U256, value: &U256);
    fn syscall_call(
        &self,
        fuel_limit: u64,
        address: Address,
        value: U256,
        input: &[u8],
    ) -> (Bytes, i32);
    fn syscall_call_code(
        &self,
        fuel_limit: u64,
        address: Address,
        value: U256,

        input: &[u8],
    ) -> (Bytes, i32);
    fn syscall_static_call(&self, fuel_limit: u64, address: Address, input: &[u8]) -> (Bytes, i32);
    fn syscall_delegate_call(
        &self,
        fuel_limit: u64,
        address: Address,
        input: &[u8],
    ) -> (Bytes, i32);
    fn syscall_create(
        &self,
        fuel_limit: u64,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> Result<Address, i32>;
    fn syscall_emit_log(&self, data: &[u8], topics: &[B256]);
    fn syscall_destroy_account(&self, target: &Address);
    fn syscall_balance(&self, address: &Address) -> U256;
    fn syscall_write_preimage(&self, preimage: &Bytes) -> B256;
    fn syscall_preimage_size(&self, hash: &B256) -> u32;
    fn syscall_preimage_copy(&self, hash: &B256) -> Bytes;
    fn syscall_ext_storage_read(&self, address: &Address, slot: &U256) -> U256;
    fn syscall_transient_read(&self, slot: &U256) -> U256;
    fn syscall_transient_write(&self, slot: &U256, value: &U256);
}

impl<T: NativeAPI> SyscallAPI for T {
    fn syscall_storage_read(&self, slot: &U256) -> U256 {
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_STORAGE_READ,
            slot.as_le_slice(),
            GAS_LIMIT_SYSCALL_STORAGE_READ,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn syscall_storage_write(&self, slot: &U256, value: &U256) {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_STORAGE_WRITE,
            &input,
            GAS_LIMIT_SYSCALL_STORAGE_WRITE,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
    }

    fn syscall_call(
        &self,
        fuel_limit: u64,
        address: Address,
        value: U256,
        input: &[u8],
    ) -> (Bytes, i32) {
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (_, exit_code) = self.exec(&SYSCALL_ID_CALL, &buffer, fuel_limit, STATE_MAIN);
        (self.return_data(), exit_code)
    }

    fn syscall_call_code(
        &self,
        fuel_limit: u64,
        address: Address,
        value: U256,
        input: &[u8],
    ) -> (Bytes, i32) {
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (_, exit_code) = self.exec(&SYSCALL_ID_CALL_CODE, &buffer, fuel_limit, STATE_MAIN);
        (self.return_data(), exit_code)
    }

    fn syscall_static_call(&self, fuel_limit: u64, address: Address, input: &[u8]) -> (Bytes, i32) {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (_, exit_code) = self.exec(&SYSCALL_ID_STATIC_CALL, &buffer, fuel_limit, STATE_MAIN);
        (self.return_data(), exit_code)
    }

    fn syscall_delegate_call(
        &self,
        fuel_limit: u64,
        address: Address,
        input: &[u8],
    ) -> (Bytes, i32) {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (_, exit_code) = self.exec(&SYSCALL_ID_DELEGATE_CALL, &buffer, fuel_limit, STATE_MAIN);
        (self.return_data(), exit_code)
    }

    fn syscall_create(
        &self,
        fuel_limit: u64,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> Result<Address, i32> {
        let (buffer, code_hash) = if let Some(salt) = salt {
            let buffer = alloc_slice(32 + 32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..64].copy_from_slice(salt.as_le_slice());
            buffer[64..].copy_from_slice(init_code);
            (buffer, SYSCALL_ID_CREATE2)
        } else {
            let buffer = alloc_slice(32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..].copy_from_slice(init_code);
            (buffer, SYSCALL_ID_CREATE)
        };
        let (_, exit_code) = self.exec(&code_hash, &buffer, fuel_limit, STATE_MAIN);
        if exit_code != 0 {
            return Err(exit_code);
        }
        assert_eq!(self.output_size(), 20);
        let mut buffer = [0u8; 20];
        self.read_output(&mut buffer, 0);
        Ok(Address::from(buffer))
    }

    fn syscall_emit_log(&self, data: &[u8], topics: &[B256]) {
        let mut buffer = vec![0u8; 1 + topics.len() * B256::len_bytes()];
        assert!(topics.len() <= 4);
        buffer[0] = topics.len() as u8;
        for (i, topic) in topics.iter().enumerate() {
            buffer[(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())]
                .copy_from_slice(topic.as_slice());
        }
        buffer.extend_from_slice(data);
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_EMIT_LOG,
            &buffer,
            GAS_LIMIT_SYSCALL_EMIT_LOG,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
    }

    fn syscall_destroy_account(&self, target: &Address) {
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_DESTROY_ACCOUNT,
            target.as_slice(),
            GAS_LIMIT_SYSCALL_DESTROY_ACCOUNT,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
    }

    fn syscall_balance(&self, address: &Address) -> U256 {
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_BALANCE,
            address.as_slice(),
            GAS_LIMIT_SYSCALL_BALANCE,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn syscall_write_preimage(&self, preimage: &Bytes) -> B256 {
        let (_, exit_code) =
            self.exec(&SYSCALL_ID_WRITE_PREIMAGE, preimage.as_ref(), 0, STATE_MAIN);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.read_output(&mut output, 0);
        B256::from(output)
    }

    fn syscall_preimage_size(&self, hash: &B256) -> u32 {
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_PREIMAGE_SIZE,
            hash.as_ref(),
            GAS_LIMIT_SYSCALL_PREIMAGE_SIZE,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
        let mut output: [u8; 4] = [0u8; 4];
        self.read_output(&mut output, 0);
        LittleEndian::read_u32(&output)
    }

    fn syscall_preimage_copy(&self, hash: &B256) -> Bytes {
        let (_, exit_code) = self.exec(&SYSCALL_ID_PREIMAGE_COPY, hash.as_ref(), 0, STATE_MAIN);
        assert_eq!(exit_code, 0);
        self.return_data()
    }

    fn syscall_ext_storage_read(&self, address: &Address, slot: &U256) -> U256 {
        let mut input: [u8; 20 + 32] = [0u8; 20 + 32];
        input[0..20].copy_from_slice(address.as_slice());
        input[20..52].copy_from_slice(slot.as_le_slice());
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_EXT_STORAGE_READ,
            &input,
            GAS_LIMIT_SYSCALL_EXT_STORAGE_READ,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn syscall_transient_read(&self, slot: &U256) -> U256 {
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_TRANSIENT_READ,
            slot.as_le_slice(),
            GAS_LIMIT_SYSCALL_TRANSIENT_READ,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn syscall_transient_write(&self, slot: &U256, value: &U256) {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        let (_, exit_code) = self.exec(
            &SYSCALL_ID_TRANSIENT_WRITE,
            &input,
            GAS_LIMIT_SYSCALL_TRANSIENT_WRITE,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
    }
}
