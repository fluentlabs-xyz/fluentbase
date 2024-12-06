use crate::abi::FunctionParameter;
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use serde_json::json;
use std::{fs, path::PathBuf};
use syn::{
    parse::{Parse, ParseStream},
    Data,
    DeriveInput,
    Fields,
};

pub struct SolidityAbiGenerator {
    fields: Fields,
    pub name: syn::Ident,
}

impl SolidityAbiGenerator {
    pub fn new(input: DeriveInput) -> Result<Self, syn::Error> {
        let name = input.ident.clone();

        let fields = match input.data {
            Data::Struct(ref data) => data.fields.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "SolidityABI can only be derived for structs",
                ))
            }
        };

        Ok(Self { fields, name })
    }

    fn generate_abi(&self) -> Result<serde_json::Value, syn::Error> {
        let components = self
            .fields
            .iter()
            .map(|field| {
                let name = field
                    .ident
                    .as_ref()
                    .ok_or_else(|| {
                        syn::Error::new_spanned(field, "Unnamed fields are not supported")
                    })?
                    .to_string();

                let param = FunctionParameter::from_type(&field.ty, Some(name))
                    .map_err(|e| syn::Error::new_spanned(&field.ty, e.to_string()))?;

                Ok(param.to_json())
            })
            .collect::<Result<Vec<_>, syn::Error>>()?;

        Ok(json!({
            "type": "tuple",
            "components": components
        }))
    }

    fn ensure_abi_dir() -> Result<PathBuf, syn::Error> {
        let out_dir = std::env::var("OUT_DIR").map_err(|_| {
            syn::Error::new_spanned(
                syn::Ident::new("", proc_macro2::Span::call_site()),
                "OUT_DIR environment variable not found",
            )
        })?;

        let abi_dir = PathBuf::from(&out_dir).join("solidity_abi");
        fs::create_dir_all(&abi_dir).map_err(|e| {
            syn::Error::new_spanned(
                syn::Ident::new("", proc_macro2::Span::call_site()),
                format!("Failed to create ABI directory: {}", e),
            )
        })?;

        Ok(abi_dir)
    }

    fn save_abi(&self, abi: &serde_json::Value) -> Result<(), syn::Error> {
        let abi_dir = Self::ensure_abi_dir()?;
        let file_path = abi_dir.join(format!("{}.json", self.name));

        fs::write(&file_path, serde_json::to_string_pretty(abi).unwrap()).map_err(|e| {
            syn::Error::new_spanned(&self.name, format!("Failed to write ABI file: {}", e))
        })?;

        Ok(())
    }
}

impl Parse for SolidityAbiGenerator {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let derive_input = input.parse::<DeriveInput>()?;
        Self::new(derive_input)
    }
}

impl ToTokens for SolidityAbiGenerator {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let name = &self.name;

        let abi = match self.generate_abi() {
            Ok(abi) => abi,
            Err(e) => {
                tokens.extend(e.to_compile_error());
                return;
            }
        };

        if let Err(e) = self.save_abi(&abi) {
            tokens.extend(e.to_compile_error());
            return;
        }

        let abi_str = abi.to_string();
        let const_name = format_ident!(
            "{}_SOLIDITY_ABI",
            name.to_string().to_case(Case::ScreamingSnake)
        );

        tokens.extend(quote! {
            impl #name {
                pub const SOLIDITY_ABI: &'static str = #abi_str;
            }

            #[doc = "Generated Solidity ABI"]
            pub const #const_name: &str = #abi_str;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_simple_struct() {
        let input: DeriveInput = parse_quote! {
            struct Point {
                x: u64,
                y: U256,
            }
        };

        let generator = SolidityAbiGenerator::new(input).unwrap();
        let abi = generator.generate_abi().unwrap();

        assert_eq!(
            serde_json::to_value(&abi).unwrap(),
            json!({
                "type": "tuple",
                "components": [
                    {"name": "x", "type": "uint64"},
                    {"name": "y", "type": "uint256"}
                ]
            })
        );
    }

    #[test]
    fn test_nested_struct() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("OUT_DIR", temp_dir.path());

        let point_abi = json!({
            "type": "tuple",
            "components": [
                {"name": "x", "type": "uint64"},
                {"name": "y", "type": "uint256"}
            ]
        });

        let abi_dir = temp_dir.path().join("solidity_abi");
        fs::create_dir_all(&abi_dir).unwrap();
        fs::write(
            abi_dir.join("Point.json"),
            serde_json::to_string_pretty(&point_abi).unwrap(),
        )
        .unwrap();

        let input: DeriveInput = parse_quote! {
            struct ComplexPoint {
                point: Point,
                description: String,
                active: bool,
            }
        };

        let generator = SolidityAbiGenerator::new(input).unwrap();
        let abi = generator.generate_abi().unwrap();

        assert_eq!(
            serde_json::to_value(&abi).unwrap(),
            json!({
                "type": "tuple",
                "components": [
                    {
                        "name": "point",
                        "type": "tuple",
                        "components": [
                            {"name": "x", "type": "uint64"},
                            {"name": "y", "type": "uint256"}
                        ]
                    },
                    {"name": "description", "type": "string"},
                    {"name": "active", "type": "bool"}
                ]
            })
        );
    }
}
