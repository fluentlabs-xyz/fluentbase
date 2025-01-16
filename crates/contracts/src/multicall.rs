use alloc::vec::Vec;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    bytes::Buf,
    codec::{bytes::BytesMut, encoder::SolidityABI},
    derive::Contract,
    Bytes,
    ContractContextReader,
    SharedAPI,
};

// Selector for "multicall(bytes[])" - 0xac9650d8
const MULTICALL_SELECTOR: [u8; 4] = [0xac, 0x96, 0x50, 0xd8];

#[derive(Contract)]
struct Multicall<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> Multicall<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }

    fn main(&mut self) {
        // Read full input data
        let input_length = self.sdk.input_size();
        assert!(input_length >= 4, "multicall: insufficient input length");
        let mut call_data = alloc_slice(input_length as usize);
        self.sdk.read(&mut call_data, 0);

        // Split into selector and parameters
        let (selector, params) = call_data.split_at(4);
        assert_eq!(
            selector, MULTICALL_SELECTOR,
            "multicall: invalid method selector"
        );

        // Decode parameters into Vec<Bytes>
        let data = SolidityABI::<Vec<Bytes>>::decode(&Bytes::from(params), 0)
            .unwrap_or_else(|_| panic!("multicall: can't decode input parameters"));

        // Get contract address for delegate calls
        let target_addr = self.sdk.context().contract_address();
        let mut results = Vec::with_capacity(data.len());

        // Execute each call
        for call_data in data {
            let chunk = call_data.chunk();
            let (output, exit_code) = self.sdk.delegate_call(target_addr, chunk, 0);
            assert_eq!(exit_code, 0, "multicall: delegate call failed");
            results.push(output);
        }

        // Encode results for return
        let mut buf = BytesMut::new();
        SolidityABI::encode(&(results,), &mut buf, 0)
            .unwrap_or_else(|_| panic!("multicall: can't decode input parameters"));
        let encoded_output = buf.freeze();

        // Remove offset from encoded output since caller expects only data
        let clean_output = encoded_output[32..].to_vec();
        self.sdk.write(&clean_output);
    }
}

basic_entrypoint!(Multicall);
