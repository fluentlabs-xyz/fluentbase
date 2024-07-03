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
    fn expand(&self) -> SynResult<proc_macro2::TokenStream>;
}

#[derive(Clone, Debug)]
pub struct SolidityStorage {
    items: Vec<StorageItem>,
}

impl SolidityStorage {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn expand(input: TokenStream) -> TokenStream {
        // Call Parse method for storageItems
        let storage = parse_macro_input!(input as SolidityStorage);

        let mut expanded = proc_macro2::TokenStream::new();

        for item in storage.items.iter() {
            expanded.extend(
                item.expand()
                    .unwrap_or_else(|err| abort!(err.span(), err.to_string())),
            );
        }
        let trait_definition = trait_definition();
        let expanded = quote! {
            #trait_definition
            #expanded
        };

        TokenStream::from(expanded)
    }
}
fn trait_definition() -> proc_macro2::TokenStream {
    quote! {
        pub trait StorageValue<Client, T> {
            fn get(client: &Client, key: fluentbase_sdk::U256) -> Result<T, String>;
            fn set(client: &Client, key: fluentbase_sdk::U256, value: T) -> Result<(), String>;
        }

        impl<Client, T> StorageValue<Client, T> for T
        where
            Client: fluentbase_sdk::contracts::EvmAPI,
            T: fluentbase_sdk::codec::Encoder<T> + Default,
        {
            fn get(client: &Client, key: fluentbase_sdk::U256) -> Result<T, String> {
                let header_size = T::HEADER_SIZE;

                let mut buffer = vec![];

                for i in 0.. {
                    let key = key + fluentbase_sdk::U256::from(i);
                    let input = fluentbase_sdk::contracts::EvmSloadInput { index: key };
                    let output = client.sload(input);

                    let chunk = output.value.to_be_bytes::<32>();

                    if i * 32 > header_size && chunk.iter().all(|&x| x == 0) {
                        break;
                    }
                    buffer.extend_from_slice(&chunk);
                }

                let mut decoder = fluentbase_sdk::codec::BufferDecoder::new(&buffer);
                let mut body = T::default();
                T::decode_body(&mut decoder, 0, &mut body);
                Ok(body)
            }

            fn set(client: &Client, key: fluentbase_sdk::U256, value: T) -> Result<(), String> {
                let encoded_buffer = value.encode_to_vec(0);

                let chunk_size = 32;
                let num_chunks = (encoded_buffer.len() + chunk_size - 1) / chunk_size;

                for i in 0..num_chunks {
                    let start = i * chunk_size;
                    let end = (start + chunk_size).min(encoded_buffer.len());
                    let chunk = &encoded_buffer[start..end];

                    let mut chunk_padded = [0u8; 32];
                    chunk_padded[..chunk.len()].copy_from_slice(chunk);

                    let value_u256 = fluentbase_sdk::U256::from_be_bytes(chunk_padded);
                    let input = fluentbase_sdk::contracts::EvmSstoreInput {
                        index: key + U256::from(i),
                        value: value_u256,
                    };

                    client.sstore(input);
                }

                Ok(())
            }
        }

    }
}

impl Parse for SolidityStorage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let items: Punctuated<StorageItem, Semi> =
            input.parse_terminated(StorageItem::parse, Semi)?;
        let items = items
            .into_iter()
            .enumerate()
            .map(|(index, mut item)| {
                item.slot = index;
                item
            })
            .collect();

        Ok(SolidityStorage { items })
    }
}

#[derive(Clone, Debug)]
struct StorageItem {
    slot: usize,
    kind: StorageKind,
    name: Ident,
    args: Vec<Arg>,
    output: Option<Arg>,
    key_calculation: proc_macro2::TokenStream,
}

impl Parse for StorageItem {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let kind: StorageKind = input.parse()?;
        let name: Ident = input.parse()?;
        let (args, output) = kind.parse_args();
        let key_calculation = kind.key_calculation_fn(&args);

        Ok(StorageItem {
            slot: 0,
            kind,
            name,
            args,
            output,
            key_calculation,
        })
    }
}

impl StorageItem {
    fn expand_slot(index: u64) -> proc_macro2::TokenStream {
        quote! {
            const SLOT: fluentbase_sdk::U256 = fluentbase_sdk::U256::from_limbs([#index, 0u64, 0u64, 0u64]);
        }
    }

    fn expand_new_fn() -> proc_macro2::TokenStream {
        quote! {
            pub fn new(client: &'a T) -> Self {
                Self { client }
            }
        }
    }

    fn expand_default_trait(ident: &Ident) -> proc_macro2::TokenStream {
        quote! {
            impl<'a> Default for #ident<'a, fluentbase_sdk::contracts::EvmClient> {
                fn default() -> Self {
                    Self {
                        client: &fluentbase_sdk::contracts::EvmClient {
                            address: fluentbase_sdk::contracts::PRECOMPILE_EVM,
                            fuel: u32::MAX,
                        },
                    }
                }
            }
        }
    }

    fn expand_get_fn(args: &Vec<Arg>, output: &Option<Arg>) -> proc_macro2::TokenStream {
        let arguments: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        let output = match output {
            Some(output) => output.ty.to_token_stream(),
            None => quote! { () },
        };

        let key = if arg_names.is_empty() {
            quote! { self.key() }
        } else {
            quote! { self.key(#(#arg_names),*) }
        };

        let get_args = if arguments.len() == 0 {
            quote! { &self }
        } else {
            quote! { &self, #(#arguments),* }
        };

        quote! {
            fn get(#get_args) -> #output {
                let key = #key;
                let value = #output::default();

                #output::get(self.client, key).unwrap()
            }
        }
    }

    fn expand_set_fn(args: &Vec<Arg>, output: &Option<Arg>) -> proc_macro2::TokenStream {
        let arguments: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        let output = match output {
            Some(output) => output.ty.to_token_stream(),
            None => quote! { () },
        };

        let set_args = if arguments.len() == 0 {
            quote! { &self, value: #output }
        } else {
            quote! { &self, #(#arguments),*, value: #output }
        };

        quote! {
            fn set(#set_args) {
                let key = self.key(#(#arg_names),*);
                #output::set(self.client, key, value.clone()).unwrap();
            }
        }
    }
}

impl Expandable for StorageItem {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream> {
        let name = &self.name;
        let slot = StorageItem::expand_slot(self.slot as u64);
        let client_trait = quote! {
            fluentbase_sdk::contracts::EvmAPI
        };
        let new_fn = StorageItem::expand_new_fn();

        let get_fn = StorageItem::expand_get_fn(&self.args, &self.output);
        let set_fn = StorageItem::expand_set_fn(&self.args, &self.output);
        let key_fn = &self.key_calculation;

        let default_impl = StorageItem::expand_default_trait(&self.name);

        Ok(quote! {
            pub struct #name<'a, T: #client_trait + 'a>
                {
                    client:  &'a T,
                }
                impl <'a, T: #client_trait + 'a> #name <'a, T>
                {
                    #slot
                    #new_fn
                    #get_fn
                    #set_fn
                    #key_fn
                }

                #default_impl

        })
    }
}

trait ParseArgs {
    fn parse_args(&self) -> (Vec<Arg>, Option<Arg>);
}

#[derive(Clone, Debug)]
enum StorageKind {
    Mapping(MappingStorage),
    Array(ArrayStorage),
    Primitive(PrimitiveStorage),
}

impl Parse for StorageKind {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<MappingStorage>() {
            input.advance_to(&fork);
            return Ok(StorageKind::Mapping(parsed));
        }
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<ArrayStorage>() {
            input.advance_to(&fork);
            return Ok(StorageKind::Array(parsed));
        }
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<PrimitiveStorage>() {
            input.advance_to(&fork);
            return Ok(StorageKind::Primitive(parsed));
        }

        Err(input.error("Failed to parse StorageKind"))
    }
}

impl ParseArgs for StorageKind {
    fn parse_args(&self) -> (Vec<Arg>, Option<Arg>) {
        match self {
            StorageKind::Mapping(mapping) => MappingStorage::parse_args(&mapping.ty),
            StorageKind::Array(array) => ArrayStorage::parse_args(&array.ty),
            StorageKind::Primitive(primitive) => PrimitiveStorage::parse_args(&primitive.ty),
        }
    }
}

trait KeyCalculation {
    fn key_calculation_fn(&self, args: &Vec<Arg>) -> proc_macro2::TokenStream;
}

impl KeyCalculation for StorageKind {
    fn key_calculation_fn(&self, args: &Vec<Arg>) -> proc_macro2::TokenStream {
        match self {
            StorageKind::Mapping(mapping) => mapping.key_calculation_fn(args),
            StorageKind::Array(array) => array.key_calculation_fn(args),
            StorageKind::Primitive(primitive) => primitive.key_calculation_fn(),
        }
    }
}

#[derive(Clone, Debug)]
struct Arg {
    name: Ident,
    ty: Ident,
}

impl ToTokens for Arg {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = &self.name;
        let ty = &self.ty;
        tokens.extend(quote! { #name: #ty });
    }
}

#[derive(Clone, Debug, PartialEq)]
struct MappingStorage {
    ty: TypeMapping,
}

impl MappingStorage {
    fn parse_args(input: &TypeMapping) -> (Vec<Arg>, Option<Arg>) {
        let mut args = Vec::new();
        let mut current_mapping = input;
        let mut i = 0;

        loop {
            let mut arg = Arg {
                name: Ident::new(&format!("arg{}", i), proc_macro2::Span::call_site()),
                ty: Ident::new(
                    &current_mapping.key.to_string(),
                    proc_macro2::Span::call_site(),
                ),
            };
            if let Some(key_name) = &current_mapping.key_name {
                arg.name = key_name.0.clone();
            }
            args.push(arg);
            i += 1;

            match &*current_mapping.value {
                Type::Custom(custom_value) => {
                    let output = Arg {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                    };

                    return (args, Some(output));
                }
                Type::Mapping(inner_mapping) => {
                    current_mapping = inner_mapping;
                }
                _ => {
                    return (args, None);
                }
            }
        }
    }

    fn key_calculation_fn(&self, args: &Vec<Arg>) -> proc_macro2::TokenStream {
        let arguments: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();
        let arg_len = args.len();
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

        let key_fn = quote! {
            fn key(&self, #(#arguments),*) -> fluentbase_sdk::U256 {
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
        quote! {
            #calculate_keys_fn
            #key_hash_fn
            #key_fn
        }
    }
}

impl Parse for MappingStorage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let ty: TypeMapping = input.parse()?;

        Ok(Self { ty })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ArrayStorage {
    pub ty: TypeArray,
}

impl ArrayStorage {
    fn parse_args(array: &TypeArray) -> (Vec<Arg>, Option<Arg>) {
        let mut args = Vec::new();
        let mut current_array = array;
        let mut i = 0;

        loop {
            let arg = Arg {
                name: Ident::new(&format!("arg{}", i), proc_macro2::Span::call_site()),
                ty: Ident::new("U256", proc_macro2::Span::call_site()),
            };

            args.push(arg);
            i += 1;

            match &*current_array.ty {
                Type::Array(inner_array) => {
                    current_array = inner_array;
                }
                Type::Custom(ref custom_value) => {
                    let output = Arg {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                    };
                    return (args, Some(output));
                }

                _ => return (args, None),
            }
        }
    }
    fn key_calculation_fn(&self, args: &Vec<Arg>) -> proc_macro2::TokenStream {
        let arguments: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        let key_fn = quote! {
            fn key(&self, #(#arguments),*) -> fluentbase_sdk::U256 {
                let mut key = Self::SLOT;

                #(
                    let storage_key = {
                        let mut storage_key: [u8; 32] = [0; 32];
                        LowLevelSDK::keccak256(key.as_le_slice().as_ptr(), 32, storage_key.as_mut_ptr());
                        U256::from_be_bytes(storage_key)
                    };
                    key = storage_key + #arg_names;
                )*

                key
            }
        };
        key_fn
    }
}

impl Parse for ArrayStorage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let ty: Type = input.parse()?;

        let ty: TypeArray = match ty {
            Type::Array(array) => array,
            _ => return Err(input.error("Expected an array type")),
        };

        Ok(Self { ty })
    }
}
#[derive(Clone, Debug, PartialEq)]
struct PrimitiveStorage {
    ty: Type,
}

impl PrimitiveStorage {
    fn parse_args(ty: &Type) -> (Vec<Arg>, Option<Arg>) {
        (
            vec![],
            Some(Arg {
                name: Ident::new("output", proc_macro2::Span::call_site()),
                ty: Ident::new(&ty.to_string(), proc_macro2::Span::call_site()),
            }),
        )
    }
    fn key_calculation_fn(&self) -> proc_macro2::TokenStream {
        quote! {
            fn key(&self) -> fluentbase_sdk::U256 {
                Self::SLOT
            }
        }
    }
}

impl Parse for PrimitiveStorage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let ty: Type = input
            .parse()
            .unwrap_or_else(|err| abort!(err.span(), "type expected"));

        Ok(Self { ty })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_mapping_parse_args_single_level() {
        let mapping: TypeMapping = parse_quote! {
            mapping(Address => MyStruct)
        };

        let (args, output) = MappingStorage::parse_args(&mapping);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0].name.to_string(), "arg0");
        assert_eq!(args[0].ty.to_string(), "Address");
    }

    #[test]
    fn test_mapping_parse_args_nested() {
        let mapping: TypeMapping = parse_quote! {
                    mapping(Address owner => mapping(Address => mapping(Address balances =>
        MyStruct)))         };

        let (args, output) = MappingStorage::parse_args(&mapping);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0].name.to_string(), "owner");
        assert_eq!(args[0].ty.to_string(), "Address");

        assert_eq!(args[1].name.to_string(), "arg1");
        assert_eq!(args[1].ty.to_string(), "Address");

        assert_eq!(args[2].name.to_string(), "balances");
        assert_eq!(args[2].ty.to_string(), "Address");

        assert_eq!(output.clone().unwrap().name.to_string(), "output");
        assert_eq!(output.unwrap().ty.to_string(), "MyStruct");
    }
}
