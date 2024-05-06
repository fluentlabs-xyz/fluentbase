use crate::helpers::calc_storage_key;
use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, ExitCode};

pub fn _evm_sload<CR: ContextReader>(
    address: &Address,
    slot32_offset: *const u8,
    value32_offset: *mut u8,
) -> Result<bool, ExitCode> {
    let storage_key = calc_storage_key(address, slot32_offset);
    let is_cold = LowLevelSDK::jzkt_get(storage_key.as_ptr(), 0, value32_offset);
    Ok(is_cold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::sstore::_evm_sstore;
    use fluentbase_codec::Encoder;
    use fluentbase_sdk::{ContractInput, ExecutionContext};
    use fluentbase_types::{address, Address, Bytes};

    #[test]
    fn test_sload() {
        const ADDRESS: Address = address!("0000000000000000000000000000000000000000");
        const SLOT: [u8; 32] = [1u8; 32];
        const VALUE: [u8; 32] = [2u8; 32];
        // store value using JZKT library
        let mut contract_input = ContractInput::default();
        contract_input.contract_address = ADDRESS;
        LowLevelSDK::with_test_input(contract_input.encode_to_vec(0));
        _evm_sstore::<ExecutionContext>(&ADDRESS, SLOT.as_ptr(), VALUE.as_ptr()).unwrap();
        // read value from trie using SLOAD opcode
        let mut value = [0u8; 32];
        _evm_sload::<ExecutionContext>(&ADDRESS, SLOT.as_ptr(), value.as_mut_ptr()).unwrap();
        assert_eq!(value, VALUE);
    }
}
