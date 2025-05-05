use super::types::{rust_to_sol, ConversionError, SolType};
use serde::{Deserialize, Serialize};
use syn::{DeriveInput, Type, TypePath};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    #[serde(rename = "internalType")]
    pub internal_type: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Parameter>>,
}

impl Parameter {
    /// Creates a new Parameter
    pub fn new<S: Into<String>>(sol_type: SolType, name: S) -> Self {
        Self::from_sol_type(sol_type, name)
    }

    /// Creates a parameter from derive input
    pub fn from_derive_input(input: &DeriveInput) -> Result<Self, ConversionError> {
        // Get struct fields
        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => {
                return Err(ConversionError::UnsupportedType(
                    "Only structs are supported".to_string(),
                ))
            }
        };

        // Convert fields to parameters
        let components = fields
            .iter()
            .map(|field| {
                let field_name = field
                    .ident
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                Parameter::from_rust_type(field_name, &field.ty)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            internal_type: format!("struct {}", input.ident),
            ty: "tuple".to_string(),
            name: input.ident.to_string(),
            components: Some(components),
        })
    }

    /// Creates a new Parameter from a Rust type.
    /// If registry is provided and the type is a struct, it will be looked up in the registry.
    pub fn from_rust_type<S: Into<String>>(name: S, ty: &Type) -> Result<Self, ConversionError> {
        // Otherwise, convert using standard type conversion
        let sol_type = rust_to_sol(ty)?;
        Ok(Self::from_sol_type(sol_type, name))
    }

    /// Creates a Parameter from a `SolType`.
    fn from_sol_type<S: Into<String>>(sol_type: SolType, name: S) -> Self {
        let name = name.into();
        match &sol_type {
            SolType::Struct {
                name: struct_name,
                fields,
            } => Self {
                internal_type: format!("struct {struct_name}"),
                ty: "tuple".to_string(),
                name,
                components: Some(
                    fields
                        .iter()
                        .map(|(field_name, field_type)| {
                            Self::from_sol_type(field_type.clone(), field_name.clone())
                        })
                        .collect(),
                ),
            },
            SolType::Tuple(types) => Self {
                internal_type: "tuple".to_string(),
                ty: "tuple".to_string(),
                name,
                components: Some(
                    types
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| Self::from_sol_type(ty.clone(), format!("_{i}")))
                        .collect(),
                ),
            },
            _ => Self {
                internal_type: sol_type.abi_type_internal(),
                ty: sol_type.abi_type(),
                name,
                components: None,
            },
        }
    }

    pub fn get_canonical_type(&self) -> Result<String, ConversionError> {
        if self.ty == "tuple" {
            let components = self.components.as_ref().ok_or_else(|| {
                ConversionError::UnsupportedType("Tuple without components".to_string())
            })?;

            let inner_types = components
                .iter()
                .map(Parameter::get_canonical_type)
                .collect::<Result<Vec<_>, _>>()?;

            Ok(format!("({})", inner_types.join(",")))
        } else if self.ty.ends_with("[]") {
            let base_type = &self.ty[..self.ty.len() - 2];
            Ok(format!("{base_type}[]"))
        } else {
            Ok(self.ty.clone())
        }
    }

    #[must_use]
    pub fn is_struct(&self) -> bool {
        self.internal_type.starts_with("struct")
    }
}

/// Helper function to get full path from `TypePath`
fn get_full_path(type_path: &TypePath) -> Result<String, ConversionError> {
    let mut path = String::new();
    for segment in &type_path.path.segments {
        if !path.is_empty() {
            path.push_str("::");
        }
        path.push_str(&segment.ident.to_string());
    }
    Ok(path)
}
