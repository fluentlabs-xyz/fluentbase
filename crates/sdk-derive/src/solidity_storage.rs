use proc_macro::TokenStream;
use proc_macro2::Ident;
use proc_macro_error::abort;
use quote::{quote, ToTokens};
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::Semi,
    Path,
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
    pub client: Path,
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
                    // TODO: d1r1 should we parse return value from U256 or we can use U256 for all
                    // types?
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
    fn expand_funcs(args: &[Arg]) -> proc_macro2::TokenStream {
        let arg_tokens = args.iter().map(|arg| quote! { #arg }).collect::<Vec<_>>();
        let arg_tokens = quote! {
            #( #arg_tokens ),*
        };

        let arg_names: Vec<_> = args.iter().map(|arg| &arg.name).collect();
        let arg_len = arg_names.len();
        let calculate_keys_fn = quote! {
            fn calculate_keys(&self, slot: fluentbase_sdk::U256, args: [fluentbase_sdk::U256; #arg_len]) -> fluentbase_sdk::U256 {
                let mut key = slot;
                for arg in args {
                    key = self.key_hash(key, arg);
                }
                key
            }
        };

        let key_hash_fn = quote! {
            fn key_hash(&self, slot: fluentbase_sdk::U256, key: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {
                let mut raw_storage_key: [u8; 64] = [0; 64];
                raw_storage_key[0..32].copy_from_slice(slot.as_le_slice());
                raw_storage_key[32..64].copy_from_slice(key.as_le_slice());
                let mut storage_key: [u8; 32] = [0; 32];
                LowLevelSDK::keccak256(
                    raw_storage_key.as_ptr(),
                    raw_storage_key.len() as u32,
                    storage_key.as_mut_ptr(),
                );
                fluentbase_sdk::U256::from_be_bytes(storage_key)
            }
        };

        let padding_fn = quote! {
            fn pad_to_32_bytes(&self, bytes: &[u8]) -> [u8; 32] {
                let mut array = [0u8; 32];
                let start = 32 - bytes.len();
                array[start..].copy_from_slice(bytes);
                array
            }
        };

        let key_fn = quote! {
            pub fn key(&self, #arg_tokens) -> fluentbase_sdk::U256 {
                use alloy_sol_types::SolValue;
                let args = [
                    #(
                        fluentbase_sdk::U256::from_be_bytes({
                            let bytes = &#arg_names.abi_encode_packed();
                            let mut array = [0u8; 32];
                            let start = 32 - bytes.len();
                            array[start..].copy_from_slice(bytes);
                            array
                        }),
                    )*
                ];

                self.calculate_keys(Self::SLOT, args)
            }
        };

        let get_fn = quote! {
            fn get(&self, #arg_tokens) -> fluentbase_sdk::U256 {
                let key = self.key(#(#arg_names),*);
                let input = EvmSloadInput { index: key };
                let output = self.client.sload(input);
                output.value
            }
        };
        let set_fn = quote! {
            fn set(&self, #arg_tokens, value: fluentbase_sdk::U256) {
                let key = self.key(#(#arg_names),*);
                let input = EvmSstoreInput { index: key, value };
                self.client.sstore(input);
            }
        };

        quote! {
            #calculate_keys_fn
            #key_hash_fn
            #padding_fn
            #key_fn

            #get_fn
            #set_fn
        }
    }
}

impl Expandable for WrappedTypeMapping {
    fn expand(&self, slot: usize) -> SynResult<proc_macro2::TokenStream> {
        let args = WrappedTypeMapping::parse_args(&self.type_mapping);

        let slot = slot_from_index(slot);
        let funcs = WrappedTypeMapping::expand_funcs(&args);
        let ident = &self.ident;
        let client_trait = &self.client;

        let new_fn = quote! {
            pub fn new(client: &'a T) -> Self {
                Self { client }
            }
        };

        let set_client_fn = quote! {
            pub fn set_client(&mut self, client: &'a T) {
                self.client = Some(client);
            }
        };

        let expanded = quote! {
            struct #ident<'a, T: #client_trait>
            {
                client:  &'a T,
            }
            impl <'a, T: #client_trait> #ident <'a, T>
            {
                #slot
                #new_fn
                #funcs
            }
        };
        Ok(expanded)
    }
}
impl Parse for WrappedTypeMapping {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let type_mapping: TypeMapping = input
            .parse()
            .unwrap_or_else(|err| abort!(err.span(), "type mapping expected"));
        let ident: Ident = input
            .parse()
            .unwrap_or_else(|err| abort!(err.span(), "ident expected"));

        input.parse::<syn::token::Lt>()?;
        let client: Path = input.parse()?;
        input.parse::<syn::token::Gt>()?;

        Ok(Self {
            type_mapping,
            ident,
            client,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WrappedTypeArray {
    pub type_array: TypeArray,
    pub ident: Ident,
    pub client: Path,
}

impl Expandable for WrappedTypeArray {
    fn expand(&self, index: usize) -> SynResult<proc_macro2::TokenStream> {
        let ident = &self.ident;
        let slot = slot_from_index(index);
        let client_trait = &self.client;

        let new_fn = quote! {
            pub fn new(client: &'a T) -> Self {
                Self { client }
            }
        };

        let key_hash_fn = quote! {
            fn key_hash(&self, slot: fluentbase_sdk::U256, index: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {
                let mut storage_key: [u8; 32] = [0; 32];
                LowLevelSDK::keccak256(slot.as_le_slice().as_ptr(), 32, storage_key.as_mut_ptr());
                let storage_key = U256::from_be_bytes(storage_key);
                storage_key + index
            }
        };
        // TODO: d1r1 fix key function for nested arrays [][]
        let key_fn = quote! {
            fn key(&self, index: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {
                self.key_hash(Self::SLOT, index)
            }
        };

        let get_fn = quote! {
            fn get(&self, index: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {
                let key = self.key(index);
                let input = EvmSloadInput { index: key };
                let output = self.client.sload(input);
                output.value
            }
        };
        let set_fn = quote! {
            fn set(&self, index: fluentbase_sdk::U256, value: fluentbase_sdk::U256) {
                let key = self.key(index);
                let input = EvmSstoreInput { index: key, value };
                self.client.sstore(input);
            }
        };

        let expanded = quote! {
            struct #ident<'a, T: #client_trait>
            {
                client:  &'a T,
            }
            impl <'a, T: #client_trait> #ident <'a, T> {
                #slot
                #new_fn
                #key_fn
                #key_hash_fn
                #get_fn
                #set_fn

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
        input.parse::<syn::token::Lt>()?;
        let client: Path = input.parse()?;
        input.parse::<syn::token::Gt>()?;

        Ok(Self {
            type_array,
            ident,
            client,
        })
    }
}

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
