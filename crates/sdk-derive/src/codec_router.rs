use proc_macro::TokenStream;

use convert_case::Casing;
use quote::{quote, ToTokens};
use syn::{self, FnArg, Ident, ImplItemFn, ItemImpl, LitStr, parse::Parse, parse_macro_input};

use crate::{
    utils,
    utils::{get_all_methods, get_public_methods},
};

// #[proc_macro_attribute]
pub fn derive_codec_router(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast: ItemImpl = parse_macro_input!(item as ItemImpl);
    let struct_name = &ast.self_ty;

    let all_methods = get_all_methods(&ast);
    // TODO: we need to take methods defined
    let public_methods = get_public_methods(&ast);

    let methods_to_dispatch = if ast.trait_.is_some() {
        all_methods.clone()
    } else {
        public_methods.clone()
    };

    let router_impl = derive_codec_route_method(&methods_to_dispatch);

    let expanded = quote! {
        impl #struct_name {
            #( #all_methods )*
            #router_impl
        }
    };

    TokenStream::from(expanded)
}

fn derive_codec_route_method(methods: &Vec<&ImplItemFn>) -> proc_macro2::TokenStream {
    let selectors: Vec<proc_macro2::TokenStream> = methods
        .iter()
        .filter_map(|method| {
            let selector = derive_codec_route_selector_arm(method);
            Some(selector)
        })
        .collect();

    let match_arms = if selectors.is_empty() {
        quote! {
            _ => panic!("No methods to route"),
        }
    } else {
        quote! {
            #(#selectors),*,
            _ => panic!("unknown method"),
        }
    };

    quote! {
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
                #match_arms
            }
        }
    }
}

fn derive_codec_route_selector_arm(func: &ImplItemFn) -> proc_macro2::TokenStream {
    let method_name = &func.sig.ident;
    let signature_attr = func
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("signature"))
        .expect("Method missing #[signature] attribute");

    let method_signature: LitStr = signature_attr
        .parse_args()
        .expect("Failed to parse signature attribute");

    let method_id = utils::calculate_keccak256_id(&method_signature.value());

    let args: Vec<_> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    Some(&pat_ident.ident)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    let args_expr = derive_codec_route_selector_args(&args);

    quote! {
        #method_id => {
            #args_expr
            let output = self.#method_name(#(#args),*).encode_to_vec(0);
            SDK::write(&output);
        }
    }
}

fn derive_codec_route_selector_args(args: &[&Ident]) -> proc_macro2::TokenStream {
    if args.len() == 1 {
        let arg = args[0];
        quote! {
            let #arg = Self::decode_method_input::<#arg>(&input[4..]);
        }
    } else {
        let fields: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| quote! { #arg }).collect();
        quote! {
            let (#(#args),*) = Self::decode_method_input::<(#(#args),*)>(&input[4..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::{ItemImpl, parse_quote};

    use super::*;

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
