use crate::utils::decode_method_input;
use fluentbase_core::wasm::{call::_wasm_call, create::_wasm_create};
use fluentbase_sdk::{
    alloc_slice,
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
    Bytes,
    ContextReader,
    SharedAPI,
    SovereignAPI,
};

#[derive(Contract)]
pub struct WASM<CTX: ContextReader, SDK: SovereignAPI> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SovereignAPI> WasmAPI for WASM<CTX, SDK> {}

impl<CTX: ContextReader, SDK: SovereignAPI> WASM<CTX, SDK> {
    pub fn deploy(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main(&self) {
        let input = alloc_slice(self.sdk.input_size() as usize);
        self.sdk.read(input, 0);
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
                let output = _wasm_create(&self.ctx, &self.sdk, input);
                let output = output.encode_to_vec(0);
                self.sdk.write(&output);
            }
            WASM_CALL_METHOD_ID => {
                let input = decode_method_input::<WasmCallMethodInput>(&input[4..]);
                let output = _wasm_call(&self.ctx, &self.sdk, input);
                let output = output.encode_to_vec(0);
                self.sdk.write(&output);
            }
            _ => panic!("unknown method: {}", method_id),
        }
    }
}

basic_entrypoint!(WASM);
