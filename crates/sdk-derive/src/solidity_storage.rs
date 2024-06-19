use proc_macro::TokenStream;
use proc_macro2::Ident;
use proc_macro_error::abort;
use quote::quote;
use syn::{
    parse::{discouraged::Speculative, Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::Semi,
    Result as SynResult,
};
use syn_solidity::{TypeArray, TypeMapping};

pub fn storage_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as StorageInput);

    let output = generate_code(input).unwrap_or_else(|err| abort!(err.span(), err.to_string()));

    TokenStream::from(output)
}

fn generate_code(input: StorageInput) -> SynResult<proc_macro2::TokenStream> {
    let mut expanded = proc_macro2::TokenStream::new();

    for item in input.items {
        let item_tokens = match item {
            StorageItem::Mapping(mapping) => mapping_impl(mapping)?,
            StorageItem::Array(array) => array_impl(array)?,
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
        let type_array: TypeArray = input.parse()?;
        let ident: Ident = input.parse()?;

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
        if let Ok(parsed) = fork.parse::<ExtendedTypeMapping>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Mapping(parsed));
        }

        let fork = input.fork();
        if let Ok(parsed) = fork.parse::<ExtendedTypeArray>() {
            input.advance_to(&fork);
            return Ok(StorageItem::Array(parsed));
        }

        Err(input.error("Failed to parse input as ExtendedTypeMapping or ExtendedTypeArray"))
    }
}

fn mapping_impl(mapping: ExtendedTypeMapping) -> SynResult<proc_macro2::TokenStream> {
    eprintln!(">>>>mapping_impl");
    let ident = &mapping.ident;
    let expanded = quote! {
        struct #ident {
            key: U256,
        }
    };

    Ok(expanded)
}

fn array_impl(array: ExtendedTypeArray) -> SynResult<proc_macro2::TokenStream> {
    eprintln!(">>>>array_impl");

    let expanded = quote! {
        struct ArrayStorage {
            key: U256,
        }
    };

    Ok(expanded)
}
