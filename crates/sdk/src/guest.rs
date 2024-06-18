use crate::{
    alloc_slice,
    types::EvmCallMethodOutput,
    utils::calc_storage_key,
    Account,
    AccountCheckpoint,
    AccountManager,
    ContextReader,
    ContractInput,
    IContractInput,
    LowLevelSDK,
    SharedAPI,
    SovereignAPI,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS,
};
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{Address, Bytes, Bytes32, ExitCode, B256, U256};

#[derive(Default)]
pub struct GuestAccountManager;

impl GuestAccountManager {
    pub const DEFAULT: GuestAccountManager = GuestAccountManager {};
}

impl AccountManager for GuestAccountManager {
    #[inline(always)]
    fn checkpoint(&self) -> AccountCheckpoint {
        LowLevelSDK::checkpoint()
    }

    #[inline(always)]
    fn commit(&self) {
        let mut root32: [u8; 32] = [0u8; 32];
        LowLevelSDK::commit(root32.as_mut_ptr());
    }

    #[inline(always)]
    fn rollback(&self, account_checkpoint: AccountCheckpoint) {
        LowLevelSDK::rollback(account_checkpoint);
    }

    #[inline(always)]
    fn account(&self, address: Address) -> (Account, bool) {
        let mut result = Account::new(address);
        let address_word = address.into_word();
        // code size and nonce
        let mut buffer32 = Bytes32::default();
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_NONCE_FIELD,
            buffer32.as_mut_ptr(),
            false,
        );
        result.nonce = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_BALANCE_FIELD,
            unsafe { result.balance.as_le_slice_mut().as_mut_ptr() },
            false,
        );
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
            buffer32.as_mut_ptr(),
            false,
        );
        result.rwasm_code_size = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
            result.rwasm_code_hash.as_mut_ptr(),
            false,
        );
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
            buffer32.as_mut_ptr(),
            false,
        );
        result.source_code_size = LittleEndian::read_u64(&buffer32);
        LowLevelSDK::get_leaf(
            address_word.as_ptr(),
            JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
            result.source_code_hash.as_mut_ptr(),
            false,
        );
        (result, true)
    }

    #[inline(always)]
    fn write_account(&self, account: &Account) {
        let account_address = account.address.into_word();
        let account_fields = account.get_fields();
        LowLevelSDK::update_leaf(
            account_address.as_ptr(),
            JZKT_ACCOUNT_COMPRESSION_FLAGS,
            account_fields.as_ptr(),
            32 * account_fields.len() as u32,
        );
    }

    #[inline(always)]
    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        LowLevelSDK::preimage_size(hash.as_ptr())
    }

    #[inline(always)]
    fn preimage(&self, hash: &[u8; 32]) -> Bytes {
        let preimage_size = LowLevelSDK::preimage_size(hash.as_ptr()) as usize;
        let mut preimage = vec![0u8; preimage_size];
        LowLevelSDK::preimage_copy(hash.as_ptr(), preimage.as_mut_ptr());
        preimage.into()
    }

    #[inline(always)]
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) {
        LowLevelSDK::update_preimage(
            key.as_ptr(),
            field,
            preimage.as_ptr(),
            preimage.len() as u32,
        );
    }

    #[inline(always)]
    fn storage(&self, address: Address, slot: U256, committed: bool) -> (U256, bool) {
        // TODO(dmitry123): "what if account is newly created? then result value must be zero"
        let mut value = U256::ZERO;
        let storage_key = calc_storage_key(&address, slot.as_le_slice().as_ptr());
        let is_cold = LowLevelSDK::get_leaf(
            storage_key.as_ptr(),
            0,
            unsafe { value.as_le_slice_mut().as_mut_ptr() },
            committed,
        );
        (value, is_cold)
    }

    #[inline(always)]
    fn write_storage(&self, address: Address, slot: U256, value: U256) -> bool {
        let storage_key = calc_storage_key(&address, slot.as_le_slice().as_ptr());
        LowLevelSDK::update_leaf(
            storage_key.as_ptr(),
            JZKT_STORAGE_COMPRESSION_FLAGS,
            value.as_le_slice().as_ptr() as *const [u8; 32],
            32,
        );
        true
    }

    fn log(&self, address: Address, data: Bytes, topics: &[B256]) {
        LowLevelSDK::emit_log(
            address.as_ptr(),
            // we can do such cast because B256 has transparent repr
            topics.as_ptr() as *const [u8; 32],
            topics.len() as u32 * 32,
            data.as_ptr(),
            data.len() as u32,
        );
    }

    fn exec_hash(
        &self,
        hash32_offset: *const u8,
        context: &[u8],
        input: &[u8],
        fuel_offset: *mut u32,
        state: u32,
    ) -> (Bytes, i32) {
        let exit_code = LowLevelSDK::context_call(
            hash32_offset,
            input.as_ptr(),
            input.len() as u32,
            context.as_ptr(),
            context.len() as u32,
            core::ptr::null_mut(),
            0,
            fuel_offset,
            state,
        );
        let out_size = LowLevelSDK::output_size();
        let mut output_buffer = vec![0u8; out_size as usize];
        LowLevelSDK::read_output(output_buffer.as_mut_ptr(), 0, out_size);
        (output_buffer.into(), exit_code)
    }

    fn inc_nonce(&self, account: &mut Account) -> Option<u64> {
        let old_nonce = account.nonce;
        if old_nonce == u64::MAX {
            return None;
        }
        account.nonce += 1;
        Some(old_nonce)
    }

    fn transfer(&self, from: &mut Account, to: &mut Account, value: U256) -> Result<(), ExitCode> {
        Account::transfer(from, to, value)
    }

    fn precompile(
        &self,
        _address: &Address,
        _input: &Bytes,
        _gas: u64,
    ) -> Option<EvmCallMethodOutput> {
        // in jzkt mode we don't support precompiles
        None
    }

    fn is_precompile(&self, _address: &Address) -> bool {
        // in jzkt mode we don't support precompiles
        false
    }

    fn self_destruct(&self, _address: Address, _target: Address) -> [bool; 4] {
        todo!("how we can support SELFDESTRUCT (?)")
    }

    fn block_hash(&self, _number: U256) -> B256 {
        todo!("how we can support BLOCKHASH (?)")
    }

    fn write_transient_storage(&self, _address: Address, _index: U256, _value: U256) {
        todo!("how we can support TLOAD (?)")
    }

    fn transient_storage(&self, _address: Address, _index: U256) -> U256 {
        todo!("how we can support TSTORE (?)")
    }

    fn mark_account_created(&self, _address: Address) {}
}

#[derive(Default, Copy, Clone)]
pub struct GuestContextReader;

impl GuestContextReader {
    pub const DEFAULT: GuestContextReader = GuestContextReader {};
}

macro_rules! impl_reader_helper {
    (@header $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        _ = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        result
    };
    (@dynamic $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        if length > 0 {
            let mut buffer2 = vec![0; offset + length];
            buffer2[0..<$input_type>::FIELD_SIZE].copy_from_slice(&buffer);
            let buffer3 = &mut buffer2.as_mut_slice()[offset..(offset + length)];
            LowLevelSDK::read_context(buffer3.as_mut_ptr(), offset as u32, buffer3.len() as u32);
            <$input_type>::decode_field_body_at(&buffer2, 0, &mut result);
        }
        result
    };
    (@size $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        LowLevelSDK::read_context(
            buffer.as_mut_ptr(),
            <$input_type>::FIELD_OFFSET as u32,
            buffer.len() as u32,
        );
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        (offset as u32, length as u32)
    };
}

macro_rules! impl_reader_func {
    (fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self) -> $return_typ {
                impl_reader_helper!{@header <ContractInput as IContractInput>::$input_type, $return_typ}
            }
        }
    };
    (fn $fn_name:ident(result: &mut $return_typ:ty), $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self, result: &mut $return_typ) {
                let mut buffer: [u8; <<ContractInput as IContractInput>::$input_type>::FIELD_SIZE] = [0; <<ContractInput as IContractInput>::$input_type>::FIELD_SIZE];
                LowLevelSDK::sys_context(buffer.as_mut_ptr(), <<ContractInput as IContractInput>::$input_type>::FIELD_OFFSET as u32, buffer.len() as u32);
                _ = <<ContractInput as IContractInput>::$input_type>::decode_field_header_at(&buffer, 0, result);
            }
        }
    };
    (@dynamic fn $fn_name:ident() -> $return_typ:ty, $input_type:ty) => {
        paste::paste! {
            #[inline(always)]
            fn $fn_name(&self) -> $return_typ {
                impl_reader_helper!{@dynamic <ContractInput as IContractInput>::$input_type, $return_typ}
            }
            #[inline(always)]
            fn [<$fn_name _size>](&self) -> (u32, u32) {
                impl_reader_helper!{@size <ContractInput as IContractInput>::$input_type, $return_typ}
            }
        }
    };
}

impl ContextReader for GuestContextReader {
    // block info
    impl_reader_func!(fn block_chain_id() -> u64, BlockChainId);
    impl_reader_func!(fn block_coinbase() -> Address, BlockCoinbase);
    impl_reader_func!(fn block_timestamp() -> u64, BlockTimestamp);
    impl_reader_func!(fn block_number() -> u64, BlockNumber);
    impl_reader_func!(fn block_difficulty() -> u64, BlockDifficulty);
    impl_reader_func!(fn block_gas_limit() -> u64, BlockGasLimit);
    impl_reader_func!(fn block_base_fee() -> U256, BlockBaseFee);
    // tx info
    impl_reader_func!(fn tx_gas_limit() -> u64, TxGasLimit);
    impl_reader_func!(fn tx_nonce() -> u64, TxNonce);
    impl_reader_func!(fn tx_gas_price() -> U256, TxGasPrice);
    impl_reader_func!(fn tx_gas_priority_fee() -> Option<U256>, TxGasPriorityFee);
    impl_reader_func!(fn tx_caller() -> Address, TxCaller);
    impl_reader_func!(fn tx_access_list() -> Vec<(Address, Vec<U256>)>, TxAccessList);
    impl_reader_func!(@dynamic fn tx_blob_hashes() -> Vec<B256>, TxBlobHashes);
    impl_reader_func!(fn tx_max_fee_per_blob_gas() -> Option<U256>, TxMaxFeePerBlobGas);
    // contract info
    impl_reader_func!(fn contract_gas_limit() -> u64, ContractGasLimit);
    impl_reader_func!(fn contract_address() -> Address, ContractAddress);
    impl_reader_func!(fn contract_caller() -> Address, ContractCaller);
    impl_reader_func!(fn contract_value() -> U256, ContractValue);
    impl_reader_func!(fn contract_is_static() -> bool, ContractIsStatic);
}

impl GuestContextReader {
    pub fn contract_input<'a>() -> &'a [u8] {
        let input_size = LowLevelSDK::input_size();
        let input = alloc_slice(input_size as usize);
        LowLevelSDK::read(input.as_mut_ptr(), input_size, 0);
        input
    }
}
