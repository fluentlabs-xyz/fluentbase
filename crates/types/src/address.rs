use crate::{b256, Address, NativeAPI, B256, F254, U256};
use revm_primitives::{Bytecode, Eip7702Bytecode};

const POSEIDON_DOMAIN: F254 =
    b256!("0000000000000000000000000000000000000000000000010000000000000000");

#[inline(always)]
pub fn calc_storage_key<API: NativeAPI>(address: &Address, slot32_le_ptr: *const u8) -> B256 {
    let mut slot0 = B256::ZERO;
    let mut slot1 = B256::ZERO;
    // split slot32 into two 16 byte values (slot is always 32 bytes)
    unsafe {
        core::ptr::copy(slot32_le_ptr.offset(0), slot0.as_mut_ptr(), 16);
        core::ptr::copy(slot32_le_ptr.offset(16), slot1.as_mut_ptr(), 16);
    }
    // pad address to 32 bytes value (11 bytes to avoid 254-bit overflow)
    let mut address32 = B256::ZERO;
    address32[11..31].copy_from_slice(address.as_slice());
    // compute a storage key, where formula is `p(address, p(slot_0, slot_1))`
    let storage_key = API::poseidon_hash(&slot0, &slot1, &POSEIDON_DOMAIN);
    let storage_key = API::poseidon_hash(&address32, &storage_key, &POSEIDON_DOMAIN);
    storage_key
}

#[inline(always)]
pub fn calc_create_address<API: NativeAPI>(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    Encodable::encode(&nonce, &mut &mut out[22..]);
    let out = &out[..len];
    Address::from_word(API::keccak256(&out))
}

#[inline(always)]
pub fn calc_create2_address<API: NativeAPI>(
    deployer: &Address,
    salt: &U256,
    init_code_hash: &B256,
) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    let hash = API::keccak256(&bytes);
    Address::from_word(hash)
}

fn create_eip7702_proxy_bytecode(impl_address: Address) -> Bytecode {
    let eip7702_bytecode = Eip7702Bytecode::new(impl_address);
    Bytecode::Eip7702(eip7702_bytecode)
}

// #[allow(unused)]
// fn create_rwasm_proxy_bytecode(impl_address: Address) -> Bytes {
//     let mut memory_section = vec![0u8; 32 + 20];
//     //  0..32: code hash
//     // 32..52: precompile address
//     memory_section[0..32].copy_from_slice(SYSCALL_ID_DELEGATE_CALL.as_slice()); // 32 bytes
//     memory_section[32..52].copy_from_slice(impl_address.as_slice()); // 20 bytes
//     debug_assert_eq!(memory_section.len(), 52);
//     let code_section = instruction_set! {
//         // alloc default memory
//         I32Const(1) // number of pages (64kB memory in total)
//         MemoryGrow // grow memory
//         Drop // drop exit code (it can't fail here)
//         // initializes a memory segment
//         I32Const(0) // destination
//         I32Const(0) // source
//         I32Const(memory_section.len() as u32) // length
//         MemoryInit(0) // initialize 0 segment
//         DataDrop(0) // mark 0 segment as dropped (required to satisfy WASM standards)
//         // copy input (EVM bytecode can't exceed 2*24kB, so this op is safe)
//         I32Const(52) // target
//         I32Const(SharedContextInputV1::FLUENT_HEADER_SIZE as u32) // offset
//         Call(SysFuncIdx::INPUT_SIZE) // length=input_size-header_size
//         I32Const(SharedContextInputV1::FLUENT_HEADER_SIZE as u32)
//         I32Sub
//         Call(SysFuncIdx::READ_INPUT)
//         // delegate call
//         I32Const(0) // hash32_ptr
//         I32Const(32) // input_ptr
//         Call(SysFuncIdx::INPUT_SIZE) // input_len=input_size-header_size+20
//         I32Const(SharedContextInputV1::FLUENT_HEADER_SIZE as u32)
//         I32Sub
//         I32Const(20)
//         I32Add
//         I32Const(0) // fuel_limit
//         Call(SysFuncIdx::STATE) // state
//         Call(SysFuncIdx::EXEC)
//         // forward return data into output
//         I32Const(0) // offset
//         Call(SysFuncIdx::OUTPUT_SIZE) // length
//         Call(SysFuncIdx::FORWARD_OUTPUT)
//         // exit with the resulting exit code
//         Call(SysFuncIdx::EXIT)
//     };
//     let func_section = vec![code_section.len() as u32];
//     let evm_loader_module = RwasmModule {
//         code_section,
//         memory_section,
//         func_section,
//         ..Default::default()
//     };
//     let mut rwasm_bytecode = Vec::new();
//     evm_loader_module
//         .write_binary_to_vec(&mut rwasm_bytecode)
//         .unwrap();
//     rwasm_bytecode.into()
// }

pub fn create_delegate_proxy_bytecode(impl_address: Address) -> Bytecode {
    create_eip7702_proxy_bytecode(impl_address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BytecodeOrHash;
    use alloy_primitives::{address, keccak256};

    struct TestContext;

    impl NativeAPI for TestContext {
        fn keccak256(data: &[u8]) -> B256 {
            keccak256(data)
        }

        fn sha256(_data: &[u8]) -> B256 {
            todo!()
        }

        fn poseidon(_data: &[u8]) -> F254 {
            todo!()
        }

        fn poseidon_hash(_fa: &F254, _fb: &F254, _fd: &F254) -> F254 {
            todo!()
        }

        fn ec_recover(_digest: &B256, _sig: &[u8; 64], _rec_id: u8) -> [u8; 65] {
            todo!()
        }

        fn debug_log(_message: &str) {
            todo!()
        }

        fn read(&self, _target: &mut [u8], _offset: u32) {
            todo!()
        }

        fn input_size(&self) -> u32 {
            todo!()
        }

        fn write(&self, _value: &[u8]) {
            todo!()
        }

        fn forward_output(&self, _offset: u32, _len: u32) {
            todo!()
        }

        fn exit(&self, _exit_code: i32) -> ! {
            todo!()
        }

        fn output_size(&self) -> u32 {
            todo!()
        }

        fn read_output(&self, _target: &mut [u8], _offset: u32) {
            todo!()
        }

        fn state(&self) -> u32 {
            todo!()
        }

        fn fuel(&self) -> u64 {
            todo!()
        }

        fn charge_fuel(&self, _value: u64) -> u64 {
            todo!()
        }

        fn exec<I: Into<BytecodeOrHash>>(
            &self,
            _code_hash: I,
            _input: &[u8],
            _fuel_limit: Option<u64>,
            _state: u32,
        ) -> (u64, i64, i32) {
            todo!()
        }

        fn resume(
            &self,
            _call_id: u32,
            _return_data: &[u8],
            _exit_code: i32,
            _fuel_consumed: u64,
            _fuel_refunded: i64,
        ) -> (u64, i64, i32) {
            todo!()
        }

        fn preimage_size(&self, _hash: &B256) -> u32 {
            todo!()
        }

        fn preimage_copy(&self, _hash: &B256, _target: &mut [u8]) {
            todo!()
        }
    }

    #[test]
    fn test_create_address() {
        for (address, nonce) in [
            (address!("0000000000000000000000000000000000000000"), 0),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MIN,
            ),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MAX,
            ),
            (address!("2340820934820934820934809238402983400000"), 0),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MIN,
            ),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MAX,
            ),
        ] {
            assert_eq!(
                calc_create_address::<TestContext>(&address, nonce as u64),
                address.create(nonce as u64)
            );
        }
    }

    #[test]
    fn test_create2_address() {
        let address = Address::ZERO;
        for (salt, hash) in [(
            b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
        )] {
            assert_eq!(
                calc_create2_address::<TestContext>(&address, &salt.into(), &hash),
                address.create2(salt, hash)
            );
        }
    }
}
