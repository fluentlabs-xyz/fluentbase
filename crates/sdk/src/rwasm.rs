use crate::{
    alloc_slice,
    bindings::{
        _charge_fuel,
        _checkpoint,
        _commit,
        _compute_root,
        _debug_log,
        _ecrecover,
        _emit_log,
        _exec,
        _exit,
        _forward_output,
        _get_leaf,
        _input_size,
        _keccak256,
        _output_size,
        _poseidon,
        _poseidon_hash,
        _preimage_copy,
        _preimage_size,
        _read,
        _read_context,
        _read_output,
        _rollback,
        _state,
        _update_leaf,
        _update_preimage,
        _write,
    },
    Address,
    Bytes,
    ContractInput,
    IContractInput,
    B256,
    U256,
};
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{
    calc_storage_key,
    Account,
    AccountCheckpoint,
    AccountStatus,
    Bytes32,
    ContextReader,
    ExitCode,
    Fuel,
    SharedAPI,
    SovereignAPI,
    F254,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS,
};

#[derive(Default, Copy, Clone)]
pub struct RwasmContextReader;

macro_rules! impl_reader_helper {
    (@header $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        unsafe {
            _read_context(
                buffer.as_mut_ptr(),
                <$input_type>::FIELD_OFFSET as u32,
                buffer.len() as u32,
            );
        }
        let mut result: $return_typ = Default::default();
        _ = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        result
    };
    (@dynamic $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        unsafe {
            _read_context(
                buffer.as_mut_ptr(),
                <$input_type>::FIELD_OFFSET as u32,
                buffer.len() as u32,
            );
        }
        let mut result: $return_typ = Default::default();
        let (offset, length) = <$input_type>::decode_field_header_at(&buffer, 0, &mut result);
        if length > 0 {
            let mut buffer2 = vec![0; offset + length];
            buffer2[0..<$input_type>::FIELD_SIZE].copy_from_slice(&buffer);
            let buffer3 = &mut buffer2.as_mut_slice()[offset..(offset + length)];
            unsafe {
                _read_context(buffer3.as_mut_ptr(), offset as u32, buffer3.len() as u32);
            }
            <$input_type>::decode_field_body_at(&buffer2, 0, &mut result);
        }
        result
    };
    (@size $input_type:ty, $return_typ:ty) => {
        let mut buffer: [u8; <$input_type>::FIELD_SIZE] = [0; <$input_type>::FIELD_SIZE];
        unsafe {
            _read_context(
                buffer.as_mut_ptr(),
                <$input_type>::FIELD_OFFSET as u32,
                buffer.len() as u32,
            );
        }
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
                unsafe {
                    _read_context(buffer.as_mut_ptr(), <<ContractInput as IContractInput>::$input_type>::FIELD_OFFSET as u32, buffer.len() as u32);
                }
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

impl ContextReader for RwasmContextReader {
    // block info
    impl_reader_func!(fn block_chain_id() -> u64, BlockChainId);
    impl_reader_func!(fn block_coinbase() -> Address, BlockCoinbase);
    impl_reader_func!(fn block_timestamp() -> u64, BlockTimestamp);
    impl_reader_func!(fn block_number() -> u64, BlockNumber);
    impl_reader_func!(fn block_difficulty() -> u64, BlockDifficulty);
    impl_reader_func!(fn block_prevrandao() -> B256, BlockPrevrandao);
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

#[derive(Default)]
pub struct RwasmContext;

impl SharedAPI for RwasmContext {
    #[inline(always)]
    fn keccak256(data: &[u8]) -> B256 {
        unsafe {
            let mut res = B256::ZERO;
            _keccak256(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }

    #[inline(always)]
    fn poseidon(data: &[u8]) -> F254 {
        unsafe {
            let mut res = B256::ZERO;
            _poseidon(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }

    #[inline(always)]
    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254 {
        let mut res = B256::ZERO;
        unsafe {
            _poseidon_hash(
                fa.as_ptr(),
                fb.as_ptr(),
                fd.as_ptr(),
                res.as_mut_slice().as_mut_ptr(),
            )
        }
        res
    }

    #[inline(always)]
    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        unsafe {
            let mut res: [u8; 65] = [0u8; 65];
            _ecrecover(
                digest.0.as_ptr(),
                sig.as_ptr(),
                res.as_mut_ptr(),
                rec_id as u32,
            );
            res
        }
    }

    #[inline(always)]
    fn read(&self, target: &mut [u8], offset: u32) {
        unsafe { _read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn input_size(&self) -> u32 {
        unsafe { _input_size() }
    }

    #[inline(always)]
    fn write(&self, value: &[u8]) {
        unsafe { _write(value.as_ptr(), value.len() as u32) }
    }

    #[inline(always)]
    fn forward_output(&self, offset: u32, len: u32) {
        unsafe { _forward_output(offset, len) }
    }

    #[inline(always)]
    fn exit(&self, exit_code: i32) -> ! {
        unsafe { _exit(exit_code) }
    }

    #[inline(always)]
    fn output_size(&self) -> u32 {
        unsafe { _output_size() }
    }

    #[inline(always)]
    fn read_output(&self, target: &mut [u8], offset: u32) {
        unsafe { _read_output(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn state(&self) -> u32 {
        unsafe { _state() }
    }

    #[inline(always)]
    fn read_context(&self, target: &mut [u8], offset: u32) {
        unsafe { _read_context(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn charge_fuel(&self, fuel: &mut Fuel) {
        fuel.0 = unsafe { _charge_fuel(fuel.0) }
    }

    #[inline(always)]
    fn account(&self, address: &Address) -> (Account, bool) {
        let mut result = Account::new(*address);
        let address_word = address.into_word();
        // code size and nonce
        let mut buffer32 = Bytes32::default();
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_NONCE_FIELD,
                buffer32.as_mut_ptr(),
                false,
            );
        }
        result.nonce = LittleEndian::read_u64(&buffer32);
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_BALANCE_FIELD,
                result.balance.as_le_slice_mut().as_mut_ptr(),
                false,
            );
        }
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
                buffer32.as_mut_ptr(),
                false,
            );
        }
        result.rwasm_code_size = LittleEndian::read_u64(&buffer32);
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
                result.rwasm_code_hash.as_mut_ptr(),
                false,
            );
        }
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
                buffer32.as_mut_ptr(),
                false,
            );
        }
        result.source_code_size = LittleEndian::read_u64(&buffer32);
        unsafe {
            _get_leaf(
                address_word.as_ptr(),
                JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
                result.source_code_hash.as_mut_ptr(),
                false,
            );
        }
        (result, true)
    }

    #[inline(always)]
    fn preimage_size(&self, hash: &B256) -> u32 {
        unsafe { _preimage_size(hash.0.as_ptr()) }
    }

    #[inline(always)]
    fn preimage_copy(&self, target: &mut [u8], hash: &B256) {
        unsafe { _preimage_copy(hash.0.as_ptr(), target.as_mut_ptr()) }
    }

    #[inline(always)]
    fn log(&self, address: &Address, data: Bytes, topics: &[B256]) {
        unsafe {
            _emit_log(
                address.as_ptr(),
                // we can do such a cast because B256 has transparent repr
                topics.as_ptr() as *const [u8; 32],
                topics.len() as u32 * 32,
                data.as_ptr(),
                data.len() as u32,
            );
        }
    }

    #[inline(always)]
    fn system_call(&self, address: &Address, input: &[u8], fuel: &mut Fuel) -> (Bytes, ExitCode) {
        let address32 = address.into_word();
        let hash32 = unsafe {
            let mut hash32: [u8; 32] = [0u8; 32];
            _get_leaf(
                address32.0.as_ptr(),
                JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
                hash32.as_mut_ptr(),
                false,
            );
            hash32
        };
        let mut fuel_value = fuel.0 as u32;
        let exit_code = unsafe {
            _exec(
                hash32.as_ptr(),
                core::ptr::null(),
                input.as_ptr(),
                input.len() as u32,
                core::ptr::null_mut(),
                0,
                core::ptr::null_mut(),
                0,
                &mut fuel_value as *mut u32,
            )
        };
        fuel.0 = fuel_value as u64;
        let output_size = unsafe { _output_size() };
        let output = alloc_slice(output_size as usize);
        unsafe {
            _read_output(output.as_mut_ptr(), 0, output_size);
        }
        (Bytes::copy_from_slice(output), ExitCode::from(exit_code))
    }

    #[inline(always)]
    fn debug(&self, msg: &[u8]) {
        unsafe { _debug_log(msg.as_ptr(), msg.len() as u32) }
    }
}

impl SovereignAPI for RwasmContext {
    #[inline(always)]
    fn checkpoint(&self) -> AccountCheckpoint {
        unsafe { _checkpoint() }
    }

    #[inline(always)]
    fn commit(&self) {
        let mut root32: [u8; 32] = [0u8; 32];
        unsafe { _commit(root32.as_mut_ptr()) }
    }

    #[inline(always)]
    fn rollback(&self, checkpoint: AccountCheckpoint) {
        unsafe { _rollback(checkpoint) }
    }

    #[inline(always)]
    fn write_account(&self, account: &Account, _status: AccountStatus) {
        let account_address = account.address.into_word();
        let account_fields = account.get_fields();
        unsafe {
            _update_leaf(
                account_address.as_ptr(),
                JZKT_ACCOUNT_COMPRESSION_FLAGS,
                account_fields.as_ptr(),
                32 * account_fields.len() as u32,
            );
        }
    }

    #[inline(always)]
    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) {
        unsafe {
            _update_preimage(
                key.as_ptr(),
                field,
                preimage.as_ptr(),
                preimage.len() as u32,
            );
        }
    }

    #[inline(always)]
    fn context_call(
        &self,
        address: &Address,
        input: &[u8],
        context: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> (Bytes, ExitCode) {
        let address32 = address.into_word();
        let hash32 = unsafe {
            let mut hash32: [u8; 32] = [0u8; 32];
            _get_leaf(
                address32.0.as_ptr(),
                JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
                hash32.as_mut_ptr(),
                false,
            );
            hash32
        };
        let mut fuel_value = fuel.0 as u32;
        let exit_code = unsafe {
            _exec(
                hash32.as_ptr(),
                address.as_ptr(),
                input.as_ptr(),
                input.len() as u32,
                context.as_ptr(),
                context.len() as u32,
                core::ptr::null_mut(),
                0,
                &mut fuel_value as *mut u32,
            )
        };
        fuel.0 = fuel_value as u64;
        let output: Bytes = unsafe {
            let output_size = _output_size();
            let output = alloc_slice(output_size as usize);
            _read_output(output.as_mut_ptr(), 0, output_size);
            Bytes::copy_from_slice(output)
        };
        (output, ExitCode::from(exit_code))
    }

    #[inline(always)]
    fn storage(&self, address: &Address, slot: &U256, committed: bool) -> (U256, bool) {
        // TODO(dmitry123): "what if account is newly created? then result value must be zero"
        let mut value = U256::ZERO;
        let storage_key = calc_storage_key::<Self>(&address, slot.as_le_slice().as_ptr());
        let is_cold = unsafe {
            _get_leaf(
                storage_key.as_ptr(),
                0,
                value.as_le_slice_mut().as_mut_ptr(),
                committed,
            )
        };
        (value, is_cold)
    }

    #[inline(always)]
    fn write_storage(&self, _address: &Address, _slot: &U256, _value: &U256) -> bool {
        // let storage_key = calc_storage_key(&address, slot.as_le_slice().as_ptr());
        // unsafe {
        //     _update_leaf(
        //         storage_key.as_ptr(),
        //         JZKT_STORAGE_COMPRESSION_FLAGS,
        //         value.as_le_slice().as_ptr() as *const [u8; 32],
        //         32,
        //     );
        // }
        true
    }

    #[inline(always)]
    fn write_log(&self, address: &Address, data: &Bytes, topics: &[B256]) {
        unsafe {
            _emit_log(
                address.0.as_slice().as_ptr(),
                // we can do such a cast because B256 has transparent repr
                topics.as_ptr() as *const [u8; 32],
                topics.len() as u32 * 32,
                data.as_ptr(),
                data.len() as u32,
            );
        }
    }

    fn precompile(
        &self,
        _address: &Address,
        _input: &Bytes,
        _gas: u64,
    ) -> Option<(Bytes, ExitCode, u64, i64)> {
        None
    }

    fn is_precompile(&self, _address: &Address) -> bool {
        false
    }

    fn transfer(&self, from: &mut Account, to: &mut Account, value: U256) -> Result<(), ExitCode> {
        Account::transfer(from, to, value)?;
        self.write_account(from, AccountStatus::Transfer);
        self.write_account(to, AccountStatus::Transfer);
        Ok(())
    }

    #[inline(always)]
    fn self_destruct(&self, _address: Address, _target: Address) -> [bool; 4] {
        todo!("how we can support SELFDESTRUCT (?)")
    }

    #[inline(always)]
    fn block_hash(&self, _number: U256) -> B256 {
        todo!("how we can support BLOCKHASH (?)")
    }

    #[inline(always)]
    fn write_transient_storage(&self, _address: Address, _index: U256, _value: U256) {
        todo!("how we can support TLOAD (?)")
    }

    #[inline(always)]
    fn transient_storage(&self, _address: Address, _index: U256) -> U256 {
        todo!("how we can support TSTORE (?)")
    }
}
