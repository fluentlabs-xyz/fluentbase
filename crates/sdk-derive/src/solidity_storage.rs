use proc_macro::TokenStream;
use proc_macro2::Ident;
use proc_macro_error::abort;
use quote::{quote, ToTokens};
use std::process::Output;
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::{Semi, Token},
    Result as SynResult,
};
use syn_solidity::{Type, TypeArray, TypeMapping};

trait Expandable {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream>;
}

pub struct SolidityStorage;
impl SolidityStorage {
    fn new() -> Self {
        Self
    }

    pub fn expand(input: TokenStream) -> TokenStream {
        // Call Parse method for storageItems
        let storage = parse_macro_input!(input as StorageItems);

        let mut expanded = proc_macro2::TokenStream::new();

        for item in storage.items.iter() {
            expanded.extend(
                item.expand()
                    .unwrap_or_else(|err| abort!(err.span(), err.to_string())),
            );
        }

        TokenStream::from(expanded)
    }
}

#[derive(Clone, Debug)]
struct StorageItems {
    items: Vec<StorageItem>,
}

impl Parse for StorageItems {
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

        Ok(StorageItems { items })
    }
}

trait IStorageItem {
    fn expand_slot(index: u64) -> proc_macro2::TokenStream {
        quote! {
            const SLOT: fluentbase_sdk::U256 = U256::from_limbs([#index, 0u64, 0u64, 0u64]);
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
            impl<'a> Default for #ident<'a, EvmClient> {
                fn default() -> Self {
                    Self {
                        client: &EvmClient {
                            address: PRECOMPILE_EVM,
                            fuel: u32::MAX,
                        },
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct StorageItem {
    kind: StorageKind,
    name: Ident,
    slot: usize,
    // args: [Arg],
    // value: V,
}

impl IStorageItem for StorageItem {}

impl Parse for StorageItem {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let kind: StorageKind = input.parse()?;
        let name: Ident = input.parse()?;
        Ok(StorageItem {
            kind,
            name,
            slot: 0,
        })
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

        let funcs = self.kind.expand()?;

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
                    #funcs
                }

                #default_impl

        })
    }
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

impl Expandable for StorageKind {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream> {
        match self {
            StorageKind::Mapping(mapping) => mapping.expand(),
            StorageKind::Array(array) => array.expand(),
            StorageKind::Primitive(primitive) => primitive.expand(),
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

#[derive(Clone, Debug, PartialEq)]
struct MappingStorage {
    ty: TypeMapping,
}

impl MappingStorage {
    fn parse_args(mapping: &TypeMapping) -> (Vec<Arg>, Option<Arg>) {
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
                    let output = Arg {
                        name: Ident::new("output", proc_macro2::Span::call_site()),
                        ty: Ident::new(&custom_value.to_string(), proc_macro2::Span::call_site()),
                        is_output: true,
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

    fn expand_get_fn(args: &Vec<Arg>, output: &Option<Arg>) -> proc_macro2::TokenStream {
        let arguments: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();

        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        let output = match output {
            Some(output) => output.ty.to_token_stream(),
            None => quote! { () },
        };

        quote! {
            fn get(&self, #(#arguments),*) -> #output {
                let key = self.key(#(#arg_names),*);
                let value = #output::default();

                value.get(self.client, key).unwrap()
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
        quote! {
            fn set(&self, #(#arguments),*, value: #output) {
                let key = self.key(#(#arg_names),*);
                value.set(self.client, key, value).unwrap();
            }
        }
    }

    fn expand_utils_fn(
        arg_tokens: &Vec<proc_macro2::TokenStream>,
        arg_names: &Vec<&Ident>,
    ) -> proc_macro2::TokenStream {
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
            fn key(&self, #(#arg_tokens),*) -> fluentbase_sdk::U256 {
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
            #padding_fn
            #key_fn
        }
    }
}

impl Expandable for MappingStorage {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream> {
        let (args, output) = MappingStorage::parse_args(&self.ty);
        let arg_tokens: Vec<proc_macro2::TokenStream> =
            args.iter().map(|arg| arg.to_token_stream()).collect();
        let arg_names: Vec<&Ident> = args.iter().map(|arg| &arg.name).collect();

        let get_fn = MappingStorage::expand_get_fn(&args, &output);
        let set_fn = MappingStorage::expand_set_fn(&args, &output);
        let utils_fn = MappingStorage::expand_utils_fn(&arg_tokens, &arg_names);

        Ok(quote! {
            #get_fn
            #set_fn
            #utils_fn
        })
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

impl Expandable for ArrayStorage {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream> {
        let key_hash_fn = quote! {
            fn key_hash(&self, slot: fluentbase_sdk::U256, index: fluentbase_sdk::U256) -> fluentbase_sdk::U256 {                 let mut storage_key: [u8; 32] = [0; 32];
                LowLevelSDK::keccak256(slot.as_le_slice().as_ptr(), 32,storage_key.as_mut_ptr());
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
                #key_fn
                #key_hash_fn
                #get_fn
                #set_fn
        };
        Ok(expanded)
    }
}
impl Parse for ArrayStorage {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let ty: Type = input.parse()?;

        let type_array: TypeArray = match ty {
            Type::Array(array) => array,
            _ => return Err(input.error("Expected an array type")),
        };

        Ok(Self { ty: type_array })
    }
}
#[derive(Clone, Debug, PartialEq)]
struct PrimitiveStorage {
    pub ty: Type,
}

impl Expandable for PrimitiveStorage {
    fn expand(&self) -> SynResult<proc_macro2::TokenStream> {
        let key_fn = quote! {
            fn key(&self) -> fluentbase_sdk::U256 {
               Self::SLOT
            }
        };

        let get_fn = quote! {
            pub fn get<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
                if V::HEADER_SIZE == 32
                    || V::HEADER_SIZE == 20
                    || V::HEADER_SIZE == 16
                    // || V::HEADER_SIZE == 8 // TODO: d1r1 we need to create more generic way to derrive is it dynamic or static. Now we are using it like a hack
                    || V::HEADER_SIZE == 4
                    || V::HEADER_SIZE == 2
                    || V::HEADER_SIZE == 1
                {
                    self.get_static::<V>()
                } else {
                    self.get_dynamic::<V>()
                }
            }

            pub fn get_static<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
                let key = self.key();
                let input = EvmSloadInput { index: key };
                let output = self.client.sload(input);
                let chunk = output.value.to_be_bytes::<32>();
                let size = match V::HEADER_SIZE {
                    32 => 0,  // uint256, int256, fixed256, ufixed256
                    20 => 12, // address
                    16 => 16, // uint128, int128, fixed128, ufixed128
                    8 => 24,  // uint64, int64, fixed64, ufixed64
                    4 => 28,  // uint32, int32, fixed32, ufixed32
                    2 => 30,  // uint16, int16, fixed16, ufixed16
                    1 => 31,  // uint8, int8, fixed8, ufixed8
                    _ => return Err("Unsupported static type".to_string()),
                };

                let chunk = &chunk[..32 - size];

                let mut decoder = BufferDecoder::new(&chunk);
                let mut body = V::default();
                V::decode_body(&mut decoder, 0, &mut body);

                Ok(body)
            }

            pub fn get_dynamic<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
                let key = self.key();

                // Load the header
                let output = self.client.sload(EvmSloadInput { index: key });
                let header_chunk = output.value.to_be_bytes::<32>();

                let mut decoder = BufferDecoder::new(&header_chunk);

                // Decode the header to get offset and length of the data
                let (header_offset, data_len) = V::decode_header(&mut decoder, 0, &mut V::default());

                // Calculate the number of chunks to load
                let chunk_size = 32;
                let num_chunks = (data_len + chunk_size - 1) / chunk_size;

                let mut buffer = Vec::with_capacity(num_chunks * chunk_size);

                // Load all chunks of data
                for i in 0..num_chunks {
                    let input = EvmSloadInput {
                        index: key + U256::from(i + (header_offset / chunk_size)),
                    };
                    let output = self.client.sload(input);
                    let chunk = output.value.to_be_bytes::<32>();

                    buffer.extend_from_slice(&chunk);
                }

                // Trim the buffer to the actual length of the data
                buffer.truncate(header_offset + data_len);

                let mut decoder = BufferDecoder::new(&buffer);
                let mut body = V::default();
                V::decode_body(&mut decoder, 0, &mut body);

                Ok(body)
            }
        };
        let set_fn = quote! {
            pub fn set<V: Encoder<V> + Debug>(&self, value: V) -> Result<(), String> {
                let key = self.key();
                let encoded_buffer = value.encode_to_vec(0);

                let chunk_size = 32;
                let num_chunks = (encoded_buffer.len() + chunk_size - 1) / chunk_size;

                for i in 0..num_chunks {
                    let start = i * chunk_size;
                    let end = (start + chunk_size).min(encoded_buffer.len());
                    let chunk = &encoded_buffer[start..end];

                    let mut chunk_padded = [0u8; 32];
                    chunk_padded[..chunk.len()].copy_from_slice(chunk);

                    let value_u256 = U256::from_be_bytes(chunk_padded);

                    let input = EvmSstoreInput {
                        index: key + U256::from(i),
                        value: value_u256,
                    };
                    self.client.sstore(input);
                }

                Ok(())
            }
        };

        let expanded = quote! {
                #key_fn

                #get_fn
                #set_fn
        };
        Ok(expanded)
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
    fn test_parse_args_single_level() {
        let mapping: TypeMapping = parse_quote! {
            mapping(Address => MyStruct)
        };

        let args = MappingStorage::parse_args(&mapping);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0].name.to_string(), "arg0");
        assert_eq!(args[0].ty.to_string(), "Address");
    }

    #[test]
    fn test_parse_args_nested() {
        let mapping: TypeMapping = parse_quote! {
            mapping(Address owner => mapping(Address users => mapping(Address balances => MyStruct)))
        };

        let args = MappingStorage::parse_args(&mapping);

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
