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

pub fn storage_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as StorageInput);

    let output = generate_code(input).unwrap_or_else(|err| abort!(err.span(), err.to_string()));

    TokenStream::from(output)
}

fn generate_code(input: StorageInput) -> SynResult<proc_macro2::TokenStream> {
    let mut expanded = proc_macro2::TokenStream::new();

    for (index, item) in input.items.iter().enumerate() {
        let item_tokens = match item {
            StorageItem::Mapping(mapping) => mapping_impl(&mapping, index)?,
            StorageItem::Array(array) => array_impl(&array, index)?,
        };
        expanded.extend(item_tokens);
    }

    Ok(expanded)
}

#[derive(Clone, Debug, PartialEq)]
struct ExtendedTypeMapping {
    pub type_mapping: TypeMapping,
    pub ident: Ident,
}

impl Parse for ExtendedTypeMapping {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let type_mapping: TypeMapping = input.parse()?;
        let ident: Ident = input.parse()?;

        Ok(Self {
            type_mapping,
            ident,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ExtendedTypeArray {
    pub type_array: TypeArray,
    pub ident: Ident,
}

impl Parse for ExtendedTypeArray {
    fn parse(input: ParseStream) -> SynResult<Self> {
        eprintln!("Parsing ExtendedTypeArray...");

        let ty: Type = input.parse()?;
        eprintln!("Parsed type: {:?}", ty);

        let type_array: TypeArray = match ty {
            Type::Array(array) => array,
            _ => return Err(input.error("Expected an array type")),
        };

        eprintln!("Parsed type_array: {:?}", type_array);
        let ident: Ident = input.parse()?;
        eprintln!("Parsed ident: {:?}", ident);

        Ok(Self { type_array, ident })
    }
}

#[derive(Clone, Debug)]
struct StorageInput {
    items: Punctuated<StorageItem, Semi>,
}

impl Parse for StorageInput {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let items = input.parse_terminated(StorageItem::parse, Semi)?;
        Ok(StorageInput { items })
    }
}

#[derive(Clone, Debug)]
enum StorageItem {
    Mapping(ExtendedTypeMapping),
    Array(ExtendedTypeArray),
}

impl Parse for StorageItem {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<ExtendedTypeArray>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Array(parsed));
        }
        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<ExtendedTypeMapping>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Mapping(parsed));
        }

        Err(input.error("Failed to parse input as ExtendedTypeMapping or ExtendedTypeArray"))
    }
}

fn mapping_impl(
    mapping: &ExtendedTypeMapping,
    index: usize,
) -> SynResult<proc_macro2::TokenStream> {
    eprintln!(">>>>mapping_impl: {:#?}", mapping.type_mapping);
    process_mapping(&mapping.type_mapping);

    // let ty_mapping = &mapping.type_mapping;
    // let key = &ty_mapping.key;
    // let value = &ty_mapping.value;
    // eprintln!("key: {:#?}", key);
    // eprintln!("value: {:#?}", value);
    //
    let ident = &mapping.ident;
    //
    // let key = &mapping.type_mapping.key;
    // let value = &mapping.type_mapping.value;

    let expanded = quote! {
        struct #ident {
            slot: U256,
        }
        impl #ident {
            const SLOT: U256 = U256::from(#index);
            fn new(slot: U256) -> Self {
                Self { slot: U256::from(#index) }
            }
        }
    };
    Ok(expanded)
}

fn process_mapping(mapping: &TypeMapping) {
    if let Some(key_name) = &mapping.key_name {
        println!("Key: {:?}", key_name);
    }

    match &*mapping.key {
        Type::Custom(custom_key) => {
            println!("Key type: {:?}", custom_key);
        }
        _ => (),
    }

    if let Some(value_name) = &mapping.value_name {
        println!("Value: {:?}", value_name);
    }

    match &*mapping.value {
        Type::Custom(custom_value) => {
            println!("Value type: {:?}", custom_value);
        }
        Type::Mapping(inner_mapping) => {
            process_mapping(inner_mapping);
        }
        _ => (),
    }
}

fn array_impl(array: &ExtendedTypeArray, index: usize) -> SynResult<proc_macro2::TokenStream> {
    eprintln!(">>>>array_impl");
    let ident = &array.ident;

    let expanded = quote! {
        struct #ident {
            slot: U256,
        }
        impl #ident {
            const SLOT: U256 = U256::from(#index);

            fn new(slot: U256) -> Self {
                Self { slot: U256::from(#index) }
            }
        }
    };
    Ok(expanded)
}
