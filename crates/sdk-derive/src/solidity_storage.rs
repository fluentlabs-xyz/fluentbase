use proc_macro::TokenStream;
use proc_macro2::Ident;
use proc_macro_error::abort;
use quote::{quote, ToTokens};
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::Semi,
    Result as SynResult,
};
use syn_solidity::{Type, TypeArray, TypeMapping};

trait Expandable {
    fn expand(&self, slot: usize) -> SynResult<proc_macro2::TokenStream>;
}

pub struct SolidityStorage;
impl SolidityStorage {
    fn new() -> Self {
        Self
    }

    pub fn expand(input: TokenStream) -> TokenStream {
        let input = parse_macro_input!(input as StorageItems);

        let output = SolidityStorage::expand_storage_input(&input)
            .unwrap_or_else(|err| abort!(err.span(), err.to_string()));

        TokenStream::from(output)
    }

    fn expand_storage_input(input: &StorageItems) -> SynResult<proc_macro2::TokenStream> {
        let mut expanded = proc_macro2::TokenStream::new();

        for (index, item) in input.items.iter().enumerate() {
            expanded.extend(item.expand(index)?);
        }

        Ok(expanded)
    }
}

#[derive(Clone, Debug)]
struct StorageItems {
    items: Punctuated<StorageItem, Semi>,
}

impl Parse for StorageItems {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let items = input.parse_terminated(StorageItem::parse, Semi)?;
        Ok(StorageItems { items })
    }
}

#[derive(Clone, Debug)]
enum StorageItem {
    Mapping(WrappedTypeMapping),
    Array(WrappedTypeArray),
}

impl Parse for StorageItem {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<WrappedTypeArray>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Array(parsed));
        }
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<WrappedTypeMapping>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Mapping(parsed));
        }

        Err(input.error("Failed to parse input as WrappedTypeMapping or WrappedTypeArray"))
    }
}

impl Expandable for StorageItem {
    fn expand(&self, slot: usize) -> SynResult<proc_macro2::TokenStream> {
        match self {
            StorageItem::Mapping(mapping) => mapping.expand(slot),
            StorageItem::Array(array) => array.expand(slot),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WrappedTypeMapping {
    pub type_mapping: TypeMapping,
    pub ident: Ident,
}

impl WrappedTypeMapping {
    fn parse_args(mapping: &TypeMapping) -> Vec<Arg> {
        let mut args = Vec::new();
        let mut current_mapping = mapping;
        let mut i = 0;

        loop {
            let mut arg = Arg {
                name: Ident::new(&format!("arg{}", i), proc_macro2::Span::call_site()),
                ty: Ident::new(
                    &current_mapping.key.to_string(),
                    proc_macro2::Span::call_site(),
                ),
                is_output: false,
            };
            if let Some(key_name) = &current_mapping.key_name {
                arg.name = key_name.0.clone();
            }
            args.push(arg);
            i += 1;

            match &*current_mapping.value {
                Type::Custom(custom_value) => {
                    // TODO: should we handle return values differently?
                    let _output = Arg {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                        is_output: true,
                    };

                    return args;
                }
                Type::Mapping(inner_mapping) => {
                    current_mapping = inner_mapping;
                }
                _ => {
                    return args;
                }
            }
        }
    }
    fn expand_key_fn(args: &[Arg]) -> proc_macro2::TokenStream {
        let mut arg_tokens = proc_macro2::TokenStream::new();
        for arg in args {
            arg_tokens.extend(quote! { #arg, });
        }

        let arg_names: Vec<_> = args.iter().map(|arg| &arg.name).collect();
        let arg_len = arg_names.len();
        let calculate_keys_fn = quote! {
            fn calculate_keys(&self, slot: fluentbase_sdk::U256, args: [fluentbase_sdk::U256; #arg_len]) -> fluentbase_sdk::U256 {
                let mut key = slot;
                for arg in args {
                    key = mapping_key(key, arg);
                }
                key
            }
        };

        quote! {
            fn key(&self, #arg_tokens) -> fluentbase_sdk::U256 {
                let args = [
                    #(
                        fluentbase_sdk::U256::from_be_bytes({
                            let bytes = &#arg_names.abi_encode();
                            let mut array = [0u8; 32];
                            array.copy_from_slice(&bytes);
                            array
                        }),
                    )*
                ];

                 self.calculate_keys(Self::SLOT, args)
            }

            #calculate_keys_fn

        }
    }
}

impl Parse for WrappedTypeMapping {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let type_mapping: TypeMapping = input
            .parse()
            .unwrap_or_else(|err| abort!(err.span(), err.to_string()));
        let ident: Ident = input
            .parse()
            .unwrap_or_else(|err| abort!(err.span(), err.to_string()));

        Ok(Self {
            type_mapping,
            ident,
        })
    }
}

impl Expandable for WrappedTypeMapping {
    fn expand(&self, slot: usize) -> SynResult<proc_macro2::TokenStream> {
        let args = WrappedTypeMapping::parse_args(&self.type_mapping);

        let slot = slot_from_index(slot);
        let key_fn = WrappedTypeMapping::expand_key_fn(&args);
        let ident = &self.ident;

        let expanded = quote! {
            struct #ident {}
            impl #ident {
                #slot

                #key_fn
            }
        };
        Ok(expanded)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WrappedTypeArray {
    pub type_array: TypeArray,
    pub ident: Ident,
}

impl WrappedTypeArray {
    fn expand_array_key_fn() -> proc_macro2::TokenStream {
        // TODO: d1r1 fix key function for nested arrays [][]
        quote! {
            fn key(&self, index: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {
                array_key(Self::SLOT, index)
            }
        }
    }
}

impl Expandable for WrappedTypeArray {
    fn expand(&self, index: usize) -> SynResult<proc_macro2::TokenStream> {
        let ident = &self.ident;
        let slot = slot_from_index(index);

        let key_fn = WrappedTypeArray::expand_array_key_fn();

        let expanded = quote! {
            struct #ident {
                slot: U256,
            }
            impl #ident {
                #slot

                #key_fn

            }
        };
        Ok(expanded)
    }
}
impl Parse for WrappedTypeArray {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let ty: Type = input.parse()?;

        let type_array: TypeArray = match ty {
            Type::Array(array) => array,
            _ => return Err(input.error("Expected an array type")),
        };

        let ident: Ident = input.parse()?;

        Ok(Self { type_array, ident })
    }
}

// TODO: move it somewhere else
fn slot_from_index(index: usize) -> proc_macro2::TokenStream {
    quote! {
        const SLOT: fluentbase_sdk::U256 = Self::u256_from_usize(#index);
        const fn u256_from_usize(value: usize) -> fluentbase_sdk::U256 {
        let mut bytes = [0u8; 32];
        let mut v = value;
        let mut i = 0;
        while v != 0 {
            bytes[31 - i] = (v & 0xff) as u8;
            v >>= 8;
            i += 1;
        };

        fluentbase_sdk::U256::from_be_bytes(bytes)
    }
    }
}

#[derive(Debug)]
struct Arg {
    name: Ident,
    ty: Ident,
    is_output: bool,
}

impl ToTokens for Arg {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = &self.name;
        let ty = &self.ty;
        tokens.extend(quote! { #name: #ty });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_parse_args_single_level() {
        let mapping: TypeMapping = parse_quote! {
            mapping(Address => MyStruct)
        };

        let args = WrappedTypeMapping::parse_args(&mapping);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0].name.to_string(), "arg0");
        assert_eq!(args[0].ty.to_string(), "Address");
    }

    #[test]
    fn test_parse_args_nested() {
        let mapping: TypeMapping = parse_quote! {
            mapping(Address owner => mapping(Address users => mapping(Address balances => MyStruct)))
        };

        let args = WrappedTypeMapping::parse_args(&mapping);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0].name.to_string(), "owner");
        assert_eq!(args[0].ty.to_string(), "Address");

        assert_eq!(args[1].name.to_string(), "users");
        assert_eq!(args[1].ty.to_string(), "Address");

        assert_eq!(args[2].name.to_string(), "balances");
        assert_eq!(args[2].ty.to_string(), "Address");
    }
    #[test]
    fn test_u256() {
        assert_eq!(
            format!("{:064x}", 1),
            "0000000000000000000000000000000000000000000000000000000000000001"
        )
    }
}
