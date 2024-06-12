use crate::utils::decode_method_input;
use fluentbase_core::wasm::{call::_wasm_call, create::_wasm_create};
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Encoder,
    contracts::WasmAPI,
    derive::Contract,
    types::{
        CoreInput,
        ICoreInput,
        WasmCallMethodInput,
        WasmCreateMethodInput,
        WASM_CALL_METHOD_ID,
        WASM_CREATE_METHOD_ID,
    },
    AccountManager,
    Bytes,
    ContextReader,
    GuestContextReader,
    SharedAPI,
};

#[derive(Contract)]
pub struct WASM<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> WasmAPI for WASM<'a, CR, AM> {}

impl<'a, CR: ContextReader, AM: AccountManager> WASM<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main<SDK: SharedAPI>(&self) {
        let input = GuestContextReader::contract_input();
        if input.len() < 4 {
            panic!("not well-formed input");
        }
        let mut method_id = 0u32;
        <CoreInput<Bytes> as ICoreInput>::MethodId::decode_field_header(
            &input[0..4],
            &mut method_id,
        );
        match method_id {
            WASM_CREATE_METHOD_ID => {
                let input = decode_method_input::<WasmCreateMethodInput>(&input[4..]);
                let output = _wasm_create(self.cr, self.am, input);
                SDK::write(&output.encode_to_vec(0));
            }
            WASM_CALL_METHOD_ID => {
                let input = decode_method_input::<WasmCallMethodInput>(&input[4..]);
                let output = _wasm_call(self.cr, self.am, input);
                SDK::write(&output.encode_to_vec(0));
            }
            _ => panic!("unknown method: {}", method_id),
        }
    }
}

basic_entrypoint!(
    WASM<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
