use crate::helpers::calc_storage_key;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_sstore(slot32_offset: *const u8, value32_offset: *const u8) -> Result<bool, ExitCode> {
    let storage_key = calc_storage_key(slot32_offset);
    LowLevelSDK::jzkt_update(
        storage_key.as_ptr(),
        0,
        value32_offset as *const [u8; 32],
        32,
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::sload::_evm_sload;
    use fluentbase_codec::Encoder;
    use fluentbase_sdk::evm::ContractInput;
    use fluentbase_types::{address, Address};

    #[test]
    fn test_sstore() {
        const ADDRESS: Address = address!("0000000000000000000000000000000000000000");
        const SLOT: [u8; 32] = [1u8; 32];
        const VALUE: [u8; 32] = [2u8; 32];
        // store value using JZKT library
        let mut contract_input = ContractInput::default();
        contract_input.contract_address = ADDRESS;
        LowLevelSDK::with_test_input(contract_input.encode_to_vec(0));
        let jzkt = LowLevelSDK::with_default_jzkt();
        jzkt.borrow_mut().store(&ADDRESS, &SLOT, &VALUE);
        // read value from trie using SLOAD
        let mut value = [0u8; 32];
        _evm_sload(SLOT.as_ptr(), value.as_mut_ptr()).unwrap();
        assert_eq!(value, VALUE);
        // write new value using SSTORE opcode
        const NEW_VALUE: [u8; 32] = [0xffu8; 32];
        _evm_sstore(SLOT.as_ptr(), NEW_VALUE.as_ptr()).unwrap();
        // read value from trie using SLOAD
        let mut value = [0u8; 32];
        _evm_sload(SLOT.as_ptr(), value.as_mut_ptr()).unwrap();
        assert_eq!(value, NEW_VALUE);
    }
}
