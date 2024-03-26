use crate::helpers::calc_storage_key;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_sload(slot32_offset: *const u8, value32_offset: *mut u8) -> Result<bool, ExitCode> {
    let storage_key = calc_storage_key(slot32_offset);
    let is_cold = LowLevelSDK::jzkt_get(storage_key.as_ptr(), 0, value32_offset);
    Ok(is_cold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_codec::Encoder;
    use fluentbase_sdk::evm::ContractInput;
    use fluentbase_types::{address, Address};

    #[test]
    fn test_sload() {
        const ADDRESS: Address = address!("0000000000000000000000000000000000000000");
        const SLOT: [u8; 32] = [1u8; 32];
        const VALUE: [u8; 32] = [2u8; 32];
        // store value using JZKT library
        let mut contract_input = ContractInput::default();
        contract_input.contract_address = ADDRESS;
        LowLevelSDK::with_test_input(contract_input.encode_to_vec(0));
        let jzkt = LowLevelSDK::with_default_jzkt();
        jzkt.borrow_mut().store(&ADDRESS, &SLOT, &VALUE);
        // read value from trie using SLOAD opcode
        let mut value = [0u8; 32];
        _evm_sload(SLOT.as_ptr(), value.as_mut_ptr()).unwrap();
        assert_eq!(value, VALUE);
    }
}
