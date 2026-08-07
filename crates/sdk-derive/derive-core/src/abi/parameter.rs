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
    pub fn from_rust_type<S: Into<String>>(name: S, ty: &Type) -> Result<Self, ConversionError> {
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
                // A struct type carries no fields at this point unless the caller already knows
                // them; `None` marks it as still to be resolved, which is what keeps an
                // unresolved struct from silently hashing as an empty tuple
                components: struct_components(fields),
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
            // FIX: Add special handling for arrays
            SolType::Array(inner) => {
                // Check if inner type is a struct to set proper internal_type
                let internal_type = match &**inner {
                    SolType::Struct {
                        name: struct_name, ..
                    } => {
                        format!("struct {struct_name}[]")
                    }
                    _ => {
                        // For non-struct arrays, use the standard internal type
                        format!("{}[]", inner.abi_type_internal())
                    }
                };

                // For arrays of structs, we need to provide components
                let components = match &**inner {
                    // Create components from struct fields
                    SolType::Struct { fields, .. } => struct_components(fields),
                    SolType::Tuple(types) => {
                        // For tuple arrays, provide tuple components
                        Some(
                            types
                                .iter()
                                .enumerate()
                                .map(|(i, ty)| Self::from_sol_type(ty.clone(), format!("_{i}")))
                                .collect(),
                        )
                    }
                    _ => None, // Primitive arrays don't need components
                };

                Self {
                    internal_type,
                    ty: format!("{}[]", inner.abi_type()),
                    name,
                    components,
                }
            }
            SolType::FixedArray(inner, size) => {
                // Similar to Array but with fixed size
                let internal_type = match &**inner {
                    SolType::Struct {
                        name: struct_name, ..
                    } => {
                        format!("struct {struct_name}[{size}]")
                    }
                    _ => {
                        format!("{}[{size}]", inner.abi_type_internal())
                    }
                };

                let components = match &**inner {
                    SolType::Struct { fields, .. } => struct_components(fields),
                    SolType::Tuple(types) => Some(
                        types
                            .iter()
                            .enumerate()
                            .map(|(i, ty)| Self::from_sol_type(ty.clone(), format!("_{i}")))
                            .collect(),
                    ),
                    _ => None,
                };

                Self {
                    internal_type,
                    ty: format!("{}[{size}]", inner.abi_type()),
                    name,
                    components,
                }
            }
            _ => Self {
                internal_type: sol_type.abi_type_internal(),
                ty: sol_type.abi_type(),
                name,
                components: None,
            },
        }
    }

    /// Canonical Solidity type of this parameter, as it appears in a function signature
    ///
    /// Tuples - including struct and tuple arrays - expand to their components, because the
    /// selector is hashed from this string and callers expand structs the same way. A struct whose
    /// components have not been resolved has no canonical form: emitting `()` for it would fix the
    /// router selector on a signature no caller can reproduce, so it is an error instead. See
    /// [`crate::abi::structs`] for how components are resolved before this point.
    pub fn get_canonical_type(&self) -> Result<String, ConversionError> {
        let (base_type, array_suffix) = split_array_suffix(&self.ty);

        if base_type != "tuple" {
            return Ok(self.ty.clone());
        }

        let components = self.components.as_ref().ok_or_else(|| {
            if self.is_struct() {
                ConversionError::UnsupportedType(format!(
                    "components of `{}` are unresolved, so the canonical type of parameter `{}` \
                     cannot be computed",
                    self.internal_type
                        .strip_prefix("struct ")
                        .unwrap_or(&self.internal_type),
                    self.name
                ))
            } else {
                ConversionError::UnsupportedType("Tuple without components".to_string())
            }
        })?;

        let inner_types = components
            .iter()
            .map(Parameter::get_canonical_type)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(format!("({}){array_suffix}", inner_types.join(",")))
    }

    #[must_use]
    pub fn is_struct(&self) -> bool {
        self.internal_type.starts_with("struct")
    }
}

/// Components of a struct type, or `None` while its fields are still unknown
///
/// A struct that reaches [`Parameter`] without fields has not been resolved against the crate's
/// `#[derive(Codec)]` definitions yet; see [`crate::abi::structs`].
fn struct_components(fields: &[(String, SolType)]) -> Option<Vec<Parameter>> {
    if fields.is_empty() {
        return None;
    }

    Some(
        fields
            .iter()
            .map(|(field_name, field_type)| {
                Parameter::from_sol_type(field_type.clone(), field_name.clone())
            })
            .collect(),
    )
}

/// Split an ABI type into its base type and the array suffixes attached to it
///
/// `tuple[3][]` -> `("tuple", "[3][]")`, `uint256` -> `("uint256", "")`
fn split_array_suffix(ty: &str) -> (&str, &str) {
    match ty.find('[') {
        Some(index) => ty.split_at(index),
        None => (ty, ""),
    }
}

#[allow(dead_code)]
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
