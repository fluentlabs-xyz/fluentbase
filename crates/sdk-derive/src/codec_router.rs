use crate::utils::{calculate_keccak256_id, get_all_methods};
use convert_case::Casing;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{self, parse::Parse, parse_macro_input, ImplItem, ImplItemFn, ItemImpl, LitStr};

pub fn derive_codec_router(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut ast: ItemImpl = parse_macro_input!(item as ItemImpl);

    let decode_method_input_impl: ImplItem = decode_method_input_impl();
    let deploy_imp: ImplItem = deploy_impl();
    let methods = get_all_methods(&ast);

    let dispatch_impl: ImplItem = dispatch_impl(&methods);

    ast.items.push(dispatch_impl);
    ast.items.push(decode_method_input_impl);
    ast.items.push(deploy_imp);

    TokenStream::from(quote! {
        #ast
    })
}

fn deploy_impl() -> ImplItem {
    syn::parse_quote! {
        pub fn deploy<SDK: SharedAPI>(&self) {
            // precompiles can't be deployed, it exists since a genesis state :(
        }
    }
}

fn decode_method_input_impl() -> ImplItem {
    syn::parse_quote! {
        fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
            let mut core_input = T::default();
            <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
            core_input
        }
    }
}

fn dispatch_impl(methods: &Vec<&ImplItemFn>) -> ImplItem {
    let selectors: Vec<_> = methods.iter().map(|method| selector_impl(method)).collect();
    syn::parse_quote! {
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
                #(#selectors),*,
                _ => panic!("unknown method"),
            }
        }
    }
}

fn selector_impl(func: &ImplItemFn) -> proc_macro2::TokenStream {
    let sig: Option<LitStr> = func.attrs.iter().find_map(|attr| {
        if attr.path().is_ident("signature") {
            attr.parse_args().ok()
        } else {
            None
        }
    });

    let method_signature = sig.expect("signature attribute is required");

    let method_id = calculate_keccak256_id(&method_signature.value());

    let method_body = &func.block;

    quote! {
        #method_id => {
            #method_body
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::{parse_quote, ItemImpl};

    #[test]
    fn test_generate_main_method() {
        let item_impl: ItemImpl = parse_quote! {
            #[router(mode="codec")]
            impl<'a, CR: ContextReader, AM: AccountManager> EVM<'a, CR, AM> {
                pub fn deploy<SDK: SharedAPI>(&self) {
                    // precompiles can't be deployed, it exists since a genesis state :(
                }

                #[signature="_evm_create(bytes,uint256,u64,bool,uint256)"]
                fn evm_create<SDK: SharedAPI>(&self, input: EvmCreateMethodInput) {
                    let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                    let output = _evm_create(self.cr, self.am, input);
                    SDK::write(&output.encode_to_vec(0));
                }

                #[signature="_evm_call(address,uint256,bytes,uint64)"]
                fn evm_call<SDK: SharedAPI>(&self, input: EvmCallMethodInput) {
                    let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                    let output = _evm_call(self.cr, self.am, input);
                    SDK::write(&output.encode_to_vec(0));
                }

                #[signature="_evm_sload(uint256)"]
                fn evm_sload<SDK: SharedAPI>(&self, input: EvmSloadMethodInput) {
                    let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
                    let value = self.sload::<SDK>(input.index);
                    let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
                    SDK::write(&output);
                }

                #[signature="_evm_sstore(uint256,uint256)"]
                fn evm_sstore<SDK: SharedAPI>(&self, input: EvmSstoreMethodInput) {
                    let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
                    self.sstore::<SDK>(input.index, input.value);
                    let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
                    SDK::write(&output);
                }

                fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
                    let mut core_input = T::default();
                    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
                    core_input
                }
            }
        };

        let expected_main = quote! {
            impl<'a, CR: ContextReader, AM: AccountManager> EVM<'a, CR, AM> {
                pub fn deploy<SDK: SharedAPI>(&self) {
                    // precompiles can't be deployed, it exists since a genesis state :(
                }

                #[signature="_evm_create(bytes,uint256,u64,bool,uint256)"]
                fn evm_create<SDK: SharedAPI>(&self, input: EvmCreateMethodInput) {
                    let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                    let output = _evm_create(self.cr, self.am, input);
                    SDK::write(&output.encode_to_vec(0));
                }

                #[signature="_evm_call(address,uint256,bytes,uint64)"]
                fn evm_call<SDK: SharedAPI>(&self, input: EvmCallMethodInput) {
                    let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                    let output = _evm_call(self.cr, self.am, input);
                    SDK::write(&output.encode_to_vec(0));
                }

                #[signature="_evm_sload(uint256)"]
                fn evm_sload<SDK: SharedAPI>(&self, input: EvmSloadMethodInput) {
                    let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
                    let value = self.sload::<SDK>(input.index);
                    let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
                    SDK::write(&output);
                }

                #[signature="_evm_sstore(uint256,uint256)"]
                fn evm_sstore<SDK: SharedAPI>(&self, input: EvmSstoreMethodInput) {
                    let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
                    self.sstore::<SDK>(input.index, input.value);
                    let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
                    SDK::write(&output);
                }

                fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
                    let mut core_input = T::default();
                    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
                    core_input
                }
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
                    EVM_CREATE_METHOD_ID => {
                        let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                        let output = _evm_create(self.cr, self.am, input);
                        SDK::write(&output.encode_to_vec(0));
                    }
                    EVM_CALL_METHOD_ID => {
                        let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                        let output = _evm_call(self.cr, self.am, input);
                        SDK::write(&output.encode_to_vec(0));
                    }
                    EVM_SLOAD_METHOD_ID => {
                        let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
                        let value = self.sload::<SDK>(input.index);
                        let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
                        SDK::write(&output);
                    }
                    EVM_SSTORE_METHOD_ID => {
                        let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
                        self.sstore::<SDK>(input.index, input.value);
                        let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
                        SDK::write(&output);
                    }
                    _ => panic!("unknown method: {}", method_id),
                }
            }
        };

        let generated_main =
            derive_codec_router(quote! {}.into(), item_impl.into_token_stream().into());

        assert_eq!(generated_main.to_string(), expected_main.to_string());
    }
}
