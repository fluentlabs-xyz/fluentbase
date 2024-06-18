use crate::utils::{calculate_keccak256_id, get_all_methods};
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    self,
    parse_macro_input,
    punctuated::Punctuated,
    FnArg,
    ImplItem,
    ImplItemFn,
    ItemImpl,
    LitStr,
    Token,
};

pub fn derive_codec_router(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast: ItemImpl = parse_macro_input!(item as ItemImpl);
    let struct_name = &ast.self_ty;
    let (impl_generics, _, _) = ast.generics.split_for_impl();

    let decode_method_input_impl: ImplItem = decode_method_input_fn_impl();
    let deploy_imp: ImplItem = deploy_fn_impl();
    let methods = get_all_methods(&ast);

    let dispatch_impl: ImplItem = main_fn_impl(&methods);

    TokenStream::from(quote! {
        #ast
        impl #impl_generics #struct_name {
            #decode_method_input_impl
            #deploy_imp
            #dispatch_impl
        }
    })
}

fn deploy_fn_impl() -> ImplItem {
    syn::parse_quote! {
        pub fn deploy<SDK: SharedAPI>(&self) {
            // precompiles can't be deployed, it exists since a genesis state :(
        }
    }
}

fn decode_method_input_fn_impl() -> ImplItem {
    syn::parse_quote! {
        fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
            let mut core_input = T::default();
            <fluentbase_sdk::types::CoreInput<T> as fluentbase_sdk::types::ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
            core_input
        }
    }
}

fn main_fn_impl(methods: &Vec<&ImplItemFn>) -> ImplItem {
    let selectors: Vec<_> = methods.iter().map(|method| selector_impl(method)).collect();
    syn::parse_quote! {
        pub fn main<SDK: SharedAPI>(&self) {
            let input = fluentbase_sdk::GuestContextReader::contract_input();
            if input.len() < 4 {
                panic!("not well-formed input");
            }
            let mut method_id = 0u32;
            <fluentbase_sdk::types::CoreInput<fluentbase_sdk::Bytes> as fluentbase_sdk::types::ICoreInput>::MethodId::decode_field_header(
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
    let method_name = &func.sig.ident;
    let method_signature = sig.expect("signature attribute is required");
    let method_id = calculate_keccak256_id(&method_signature.value());
    let input_ty = method_input_ty(&func.sig.inputs);

    quote! {
        #method_id => {
            let input = Self::decode_method_input::<#input_ty>(&input[4..]);
            let output = self.#method_name::<SDK>(input);
            let output = output.encode_to_vec(0);
            SDK::write(output.as_ptr(), output.len() as u32);
        }
    }
}

pub fn method_input_ty(inputs: &Punctuated<FnArg, Token![,]>) -> Option<proc_macro2::TokenStream> {
    inputs
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    let param_name = &pat_ident.ident;
                    if param_name == "input" {
                        let param_type = &pat_type.ty;
                        return Some(quote! { #param_type });
                    }
                }
            }
            None
        })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;
    use syn::{parse_quote, ImplItemFn};

    #[test]
    fn test_dispatch_impl() {
        let methods: Vec<ImplItemFn> = vec![
            parse_quote! {
                #[signature("_evm_create(bytes,uint256,u64,bool,uint256)")]
                fn evm_create<SDK: SharedAPI>(&self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput {
                    _evm_create(self.cr, self.am, input)
                }
            },
            parse_quote! {
                #[signature("_evm_call(address,uint256,bytes,uint64)")]
                fn evm_call<SDK: SharedAPI>(&self, input: EvmCallMethodInput) -> EvmCallMethodOutput {
                    _evm_call(self.cr, self.am, input)
                }
            },
        ];

        let method_refs: Vec<&ImplItemFn> = methods.iter().collect();
        let dispatch = main_fn_impl(&method_refs);

        let expected_dispatch: ImplItem = parse_quote! {
            pub fn main<SDK: SharedAPI>(&self) {
                let input = fluentbase_sdk::GuestContextReader::contract_input();
                if input.len() < 4 {
                    panic!("not well-formed input");
                }
                let mut method_id = 0u32;
                <fluentbase_sdk::types::CoreInput<fluentbase_sdk::Bytes> as fluentbase_sdk::types::ICoreInput>::MethodId::decode_field_header(
                    &input[0..4],
                    &mut method_id,
                );
                match method_id {
                    895509340u32 => {
                        let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                        let output = self.evm_create::<SDK>(input);
                        let output = output.encode_to_vec(0);
                        SDK::write(output.as_ptr(), output.len() as u32);
                    },
                    4246677046u32 => {
                        let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                        let output = self.evm_call::<SDK>(input);
                        let output = output.encode_to_vec(0);
                        SDK::write(output.as_ptr(), output.len() as u32);
                    },
                    _ => panic!("unknown method"),
                }
            }
        };

        assert_eq!(
            dispatch.to_token_stream().to_string(),
            expected_dispatch.to_token_stream().to_string()
        );
    }
}
