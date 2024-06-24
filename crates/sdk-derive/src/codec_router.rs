use crate::utils::{calculate_keccak256_id, get_all_methods};
use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{
    self,
    parse_macro_input,
    punctuated::Punctuated,
    FnArg,
    ImplItem,
    ImplItemFn,
    ItemImpl,
    ItemTrait,
    LitStr,
    ReturnType,
    Token,
    TraitItem,
    TraitItemFn,
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
        pub fn deploy<SDK: fluentbase_sdk::SharedAPI>(&self) {
            unreachable!("precompiles can't be deployed, it exists since a genesis state")
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
        pub fn main<SDK: fluentbase_sdk::SharedAPI>(&self) {
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
            let input = Self::decode_method_input::<#input_ty>(&input);
            let output = self.#method_name(input);
            let output = output.encode_to_vec(0);
            fluentbase_sdk::LowLevelSDK::write(output.as_ptr(), output.len() as u32);
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

pub fn derive_codec_client(_attr: TokenStream, ast: ItemTrait) -> TokenStream {
    let items = ast
        .items
        .iter()
        .filter_map(|item| {
            if let TraitItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect::<Vec<&TraitItemFn>>();

    let sdk_crate_name = if std::env::var("CARGO_PKG_NAME").unwrap() == "fluentbase-sdk" {
        quote! { crate }
    } else {
        quote! { fluentbase_sdk }
    };
    let codec_crate_name = if std::env::var("CARGO_PKG_NAME").unwrap() == "fluentbase-sdk" {
        quote! { crate::codec }
    } else {
        quote! { fluentbase_sdk::codec }
    };

    let mut methods = Vec::new();
    for item in items {
        let sig = &item.sig;
        let mut inputs = Vec::new();
        for arg in sig.inputs.iter() {
            let arg = match arg {
                FnArg::Receiver(_) => continue,
                FnArg::Typed(arg) => &arg.pat,
            };
            inputs.push(quote! { #arg });
        }
        let output_type = match &sig.output {
            ReturnType::Default => panic!("missing mandatory return type"),
            ReturnType::Type(_, ty) => ty,
        };

        let method_sig: Option<LitStr> = item
            .attrs
            .iter()
            .find_map(|attr| {
                if attr.path().is_ident("signature") {
                    attr.parse_args().ok()
                } else {
                    None
                }
            })
            .expect("missing signature attribute");
        let method_sig = quote! { #method_sig };

        let sol_sig = calculate_keccak256_id(method_sig.to_string().as_str());
        let method = quote! {
            #sig {
                use #codec_crate_name::Encoder;
                use #sdk_crate_name::types::CoreInput;
                let core_input = CoreInput {
                    method_id: #sol_sig,
                    method_data: input,
                }.encode_to_vec(0);
                let (output, exit_code) =
                    #sdk_crate_name::contracts::call_system_contract(&self.address, &core_input, self.fuel);
                if exit_code != 0 {
                    panic!("system contract call failed with exit code: {}", exit_code);
                }
                let mut decoder = #codec_crate_name::BufferDecoder::new(&output);
                let mut result = #output_type::default();
                #output_type::decode_body(&mut decoder, 0, &mut result);
                result
            }
        };
        methods.push(method);
    }

    let mut ident_name = ast.ident.to_string();
    if ident_name.ends_with("API") {
        ident_name = ident_name.trim_end_matches("API").to_string();
    }
    let client_name = Ident::new((ident_name + "Client").as_str(), ast.ident.span());
    let trait_name = &ast.ident;

    let expanded = quote! {
        #ast
        pub struct #client_name {
            pub address: #sdk_crate_name::Address,
            pub fuel: u32,
        }
        impl #client_name {
            pub fn new(address: #sdk_crate_name::Address) -> impl #trait_name {
                Self { address, fuel: u32::MAX }
            }
        }
        impl #trait_name for #client_name {
            #( #methods )*
        }
    };

    TokenStream::from(expanded)
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
                fn evm_create(&self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput {
                    _evm_create(self.cr, self.am, input)
                }
            },
            parse_quote! {
                #[signature("_evm_call(address,uint256,bytes,uint64)")]
                fn evm_call(&self, input: EvmCallMethodInput) -> EvmCallMethodOutput {
                    _evm_call(self.cr, self.am, input)
                }
            },
        ];

        let method_refs: Vec<&ImplItemFn> = methods.iter().collect();
        let dispatch = main_fn_impl(&method_refs);

        let expected_dispatch: ImplItem = parse_quote! {
            pub fn main(&self) {
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
                        fluentbase_sdk::LowLevelSDK::write(output.as_ptr(), output.len() as u32);
                    },
                    4246677046u32 => {
                        let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                        let output = self.evm_call::<SDK>(input);
                        let output = output.encode_to_vec(0);
                        fluentbase_sdk::LowLevelSDK::write(output.as_ptr(), output.len() as u32);
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
