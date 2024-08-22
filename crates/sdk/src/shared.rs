use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_types::{
    Address,
    BlockContext,
    Bytes,
    ContractContext,
    NativeAPI,
    SharedAPI,
    SharedContextInputV1,
    SyscallAPI,
    TxContext,
    B256,
    F254,
    U256,
};

pub struct SharedContextImpl<API: NativeAPI> {
    native_sdk: API,
    input: SharedContextInputV1,
}

impl<API: NativeAPI> SharedContextImpl<API> {
    pub fn parse_from_input(native_sdk: API) -> Self {
        let input_size = native_sdk.input_size() as usize;
        assert!(
            input_size >= SharedContextInputV1::HEADER_SIZE,
            "malformed input header"
        );
        let mut header_input: [u8; SharedContextInputV1::HEADER_SIZE] =
            [0u8; SharedContextInputV1::HEADER_SIZE];
        native_sdk.read(&mut header_input, 0);
        let mut buffer_decoder = BufferDecoder::new(&header_input);
        let mut result = Self {
            native_sdk,
            input: Default::default(),
        };
        SharedContextInputV1::decode_header(&mut buffer_decoder, 0, &mut result.input);
        result
    }

    pub fn commit_changes_and_exit(&mut self) -> ! {
        self.native_sdk.exit(0);
    }
}

impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn block_context(&self) -> &BlockContext {
        &self.input.block
    }

    fn tx_context(&self) -> &TxContext {
        &self.input.tx
    }

    fn contract_context(&self) -> &ContractContext {
        &self.input.contract
    }

    fn write_storage(&mut self, slot: U256, value: U256) {
        self.native_sdk.syscall_storage_write(&slot, &value);
    }

    fn storage(&self, slot: &U256) -> U256 {
        self.native_sdk.syscall_storage_read(slot)
    }

    fn ext_storage(&self, address: &Address, slot: &U256) -> U256 {
        self.native_sdk.syscall_ext_storage_read(address, slot)
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
        let preimage = self.native_sdk.syscall_preimage_copy(hash);
        target.copy_from_slice(preimage.as_ref());
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        self.native_sdk.syscall_preimage_size(hash)
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]) {
        self.native_sdk.syscall_emit_log(data.as_ref(), topics);
    }

    fn balance(&self, address: &Address) -> U256 {
        self.native_sdk.syscall_balance(address)
    }

    fn write_preimage(&mut self, preimage: Bytes) -> B256 {
        self.native_sdk.syscall_write_preimage(&preimage)
    }

    fn create(
        &self,
        mut fuel_limit: u64,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        self.native_sdk
            .syscall_create(fuel_limit, salt, value, init_code)
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
        self.native_sdk
            .syscall_call(fuel_limit, address, value, input)
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
        self.native_sdk
            .syscall_call_code(fuel_limit, address, value, input)
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
        self.native_sdk
            .syscall_delegate_call(fuel_limit, address, input)
    }

    fn static_call(&mut self, address: Address, input: &[u8], mut fuel_limit: u64) -> (Bytes, i32) {
        if fuel_limit == 0 {
            fuel_limit = self.native_sdk.fuel();
        }
        self.native_sdk
            .syscall_static_call(fuel_limit, address, input)
    }

    fn destroy_account(&mut self, address: Address) {
        self.native_sdk.syscall_destroy_account(&address);
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        self.native_sdk.keccak256(data)
    }

    fn sha256(&self, data: &[u8]) -> B256 {
        self.native_sdk.sha256(data)
    }

    fn poseidon(&self, data: &[u8]) -> F254 {
        self.native_sdk.poseidon(data)
    }
}
