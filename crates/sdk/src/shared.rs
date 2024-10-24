use crate::byteorder::{ByteOrder, LittleEndian};
use alloc::vec;
use core::cell::Cell;
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_types::{
    alloc_slice,
    Address,
    BlockContext,
    Bytes,
    ContextFreeNativeAPI,
    ContractContext,
    NativeAPI,
    SharedAPI,
    SharedContextInputV1,
    TxContext,
    B256,
    F254,
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

pub struct SharedContextImpl<API: NativeAPI> {
    native_sdk: API,
    last_fuel_consumed: Cell<u64>,
}

impl<API: NativeAPI> SharedContextImpl<API> {
    pub fn parse_from_input(native_sdk: API) -> Self {
        Self {
            native_sdk,
            last_fuel_consumed: Cell::new(0),
        }
    }

    unsafe fn shared_context_ref(&self) -> &SharedContextInputV1 {
        static mut CONTEXT: Option<SharedContextInputV1> = None;
        CONTEXT.get_or_insert_with(|| {
            let input_size = self.native_sdk.input_size() as usize;
            assert!(
                input_size >= SharedContextInputV1::HEADER_SIZE,
                "malformed input header"
            );
            let mut header_input: [u8; SharedContextInputV1::HEADER_SIZE] =
                [0u8; SharedContextInputV1::HEADER_SIZE];
            self.native_sdk.read(&mut header_input, 0);
            let mut buffer_decoder = BufferDecoder::new(&header_input);
            let mut result = SharedContextInputV1::default();
            SharedContextInputV1::decode_header(&mut buffer_decoder, 0, &mut result);
            result
        });
        CONTEXT.as_ref().unwrap()
    }

    pub fn commit_changes_and_exit(&mut self) -> ! {
        self.native_sdk.exit(0);
    }
}

impl<API: NativeAPI> ContextFreeNativeAPI for SharedContextImpl<API> {
    fn keccak256(data: &[u8]) -> B256 {
        API::keccak256(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        API::sha256(data)
    }

    fn poseidon(data: &[u8]) -> F254 {
        API::poseidon(data)
    }

    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254 {
        API::poseidon_hash(fa, fb, fd)
    }

    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        API::ec_recover(digest, sig, rec_id)
    }

    fn debug_log(message: &str) {
        API::debug_log(message)
    }
}

impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn block_context(&self) -> &BlockContext {
        unsafe { &self.shared_context_ref().block }
    }

    fn tx_context(&self) -> &TxContext {
        unsafe { &self.shared_context_ref().tx }
    }

    fn contract_context(&self) -> &ContractContext {
        unsafe { &self.shared_context_ref().contract }
    }

    fn write_storage(&mut self, slot: U256, value: U256) {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_STORAGE_WRITE,
            &input,
            GAS_LIMIT_SYSCALL_STORAGE_WRITE,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
    }

    fn storage(&self, slot: &U256) -> U256 {
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_STORAGE_READ,
            slot.as_le_slice(),
            GAS_LIMIT_SYSCALL_STORAGE_READ,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.native_sdk.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_TRANSIENT_WRITE,
            &input,
            GAS_LIMIT_SYSCALL_TRANSIENT_WRITE,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
    }

    fn transient_storage(&self, slot: &U256) -> U256 {
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_TRANSIENT_READ,
            slot.as_le_slice(),
            GAS_LIMIT_SYSCALL_TRANSIENT_READ,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.native_sdk.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn ext_storage(&self, address: &Address, slot: &U256) -> U256 {
        let mut input: [u8; 20 + 32] = [0u8; 20 + 32];
        input[0..20].copy_from_slice(address.as_slice());
        input[20..52].copy_from_slice(slot.as_le_slice());
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_EXT_STORAGE_READ,
            &input,
            GAS_LIMIT_SYSCALL_EXT_STORAGE_READ,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.native_sdk.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.native_sdk
            .read(target, SharedContextInputV1::HEADER_SIZE as u32 + offset)
    }

    fn input_size(&self) -> u32 {
        let input_size = self.native_sdk.input_size();
        assert!(
            input_size >= SharedContextInputV1::HEADER_SIZE as u32,
            "input less than context header"
        );
        input_size - SharedContextInputV1::HEADER_SIZE as u32
    }

    fn charge_fuel(&self, value: u64) {
        self.native_sdk.charge_fuel(value);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write(&mut self, output: &[u8]) {
        self.native_sdk.write(output);
    }

    fn exit(&self, exit_code: i32) -> ! {
        self.native_sdk.exit(exit_code)
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_PREIMAGE_COPY, hash.as_ref(), 0, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let preimage = self.native_sdk.return_data();
        target.copy_from_slice(preimage.as_ref());
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_PREIMAGE_SIZE,
            hash.as_ref(),
            GAS_LIMIT_SYSCALL_PREIMAGE_SIZE,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 4] = [0u8; 4];
        self.native_sdk.read_output(&mut output, 0);
        LittleEndian::read_u32(&output)
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]) {
        let mut buffer = vec![0u8; 1 + topics.len() * B256::len_bytes()];
        assert!(topics.len() <= 4);
        buffer[0] = topics.len() as u8;
        for (i, topic) in topics.iter().enumerate() {
            buffer[(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())]
                .copy_from_slice(topic.as_slice());
        }
        buffer.extend_from_slice(data.as_ref());
        let (_, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_EMIT_LOG,
            &buffer,
            GAS_LIMIT_SYSCALL_EMIT_LOG,
            STATE_MAIN,
        );
        assert_eq!(exit_code, 0);
    }

    fn balance(&self, address: &Address) -> U256 {
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_BALANCE,
            address.as_slice(),
            GAS_LIMIT_SYSCALL_BALANCE,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.native_sdk.read_output(&mut output, 0);
        U256::from_le_bytes(output)
    }

    fn write_preimage(&mut self, preimage: Bytes) -> B256 {
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_WRITE_PREIMAGE, preimage.as_ref(), 0, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
        let mut output: [u8; 32] = [0u8; 32];
        self.native_sdk.read_output(&mut output, 0);
        B256::from(output)
    }

    fn create(
        &mut self,
        mut fuel_limit: u64,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> Result<Address, i32> {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
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
        let (fuel_consumed, exit_code) = self
            .native_sdk
            .exec(&code_hash, &buffer, fuel_limit, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        if exit_code != 0 {
            return Err(exit_code);
        }
        assert_eq!(self.native_sdk.output_size(), 20);
        let mut buffer = [0u8; 20];
        self.native_sdk.read_output(&mut buffer, 0);
        Ok(Address::from(buffer))
    }

    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        mut fuel_limit: u64,
    ) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_CALL, &buffer, fuel_limit, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        (self.native_sdk.return_data(), exit_code)
    }

    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        mut fuel_limit: u64,
    ) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_CALL_CODE, &buffer, fuel_limit, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        (self.native_sdk.return_data(), exit_code)
    }

    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        mut fuel_limit: u64,
    ) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_DELEGATE_CALL, &buffer, fuel_limit, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        (self.native_sdk.return_data(), exit_code)
    }

    fn static_call(&mut self, address: Address, input: &[u8], mut fuel_limit: u64) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (fuel_consumed, exit_code) =
            self.native_sdk
                .exec(&SYSCALL_ID_STATIC_CALL, &buffer, fuel_limit, STATE_MAIN);
        self.last_fuel_consumed.set(fuel_consumed);
        (self.native_sdk.return_data(), exit_code)
    }

    fn destroy_account(&mut self, address: Address) {
        let (fuel_consumed, exit_code) = self.native_sdk.exec(
            &SYSCALL_ID_DESTROY_ACCOUNT,
            address.as_slice(),
            GAS_LIMIT_SYSCALL_DESTROY_ACCOUNT,
            STATE_MAIN,
        );
        self.last_fuel_consumed.set(fuel_consumed);
        assert_eq!(exit_code, 0);
    }

    fn last_fuel_consumed(&self) -> u64 {
        self.last_fuel_consumed.get()
    }
}
