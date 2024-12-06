use super::ABIError;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env,
    fmt::{self, Display, Formatter},
    fs,
    path::PathBuf,
};
use syn::{
    FnArg,
    GenericArgument,
    Pat,
    PatIdent,
    PatType,
    PathArguments,
    PathSegment,
    ReturnType,
    Type,
    TypeArray,
    TypePath,
    TypeSlice,
    TypeTuple,
};

const UNSUPPORTED_TYPES: &[&str] = &[
    "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Option", "Result", "Arc", "Rc", "Cell",
    "RefCell", "Mutex", "RwLock",
];

/// ABI parameter representation for Solidity function
#[derive(Debug, Clone, Serialize)]
pub struct FunctionParameter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub sol_type: SolType,
    #[serde(rename = "internalType")]
    pub internal_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<FunctionParameter>>,
    #[serde(skip)]
    pub data_location: Option<DataLocation>,
    #[serde(skip)]
    pub fn_arg: Option<ArgumentInfo>,
}

/// Function argument metadata
#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentInfo {
    /// Parameter name
    pub name: String,
    /// Parameter type
    pub ty: Type,
    /// Mutability flag
    pub is_mutable: bool,
    /// Pattern information
    pub pattern: Option<Pat>,
}

/// Represents detailed Rust parameter information
#[derive(Debug, Clone, PartialEq)]
pub struct RustParameter {
    /// Parameter name
    pub name: String,
    /// Parameter type information including reference and mutability
    pub ty: Type,
    /// Is the parameter itself mutable (e.g. `mut x: Type`)
    pub is_mutable: bool,
    /// Original pattern for complex patterns support
    pub pattern: Option<Pat>,
}

/// Data location in Solidity
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DataLocation {
    Memory,
    Storage,
    Calldata,
}

/// Represents Solidity types used in the ABI.
#[derive(Debug, Clone, PartialEq)]
pub enum SolType {
    // Primitive types
    Uint(usize),
    Int(usize),
    Address,
    Bool,
    String,
    Bytes,
    FixedBytes(usize),

    // Container types
    Array(Box<SolType>),
    FixedArray(Box<SolType>, usize),
    Tuple(Vec<SolType>),

    // User-defined types
    Struct {
        name: String,
        fields: Vec<(String, SolType)>,
    },
}

#[derive(Deserialize, Serialize)]
struct StructComponent {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    components: Vec<StructComponent>,
}

#[derive(Deserialize, Serialize)]
struct StructDefinition {
    #[serde(rename = "type")]
    type_name: String,
    components: Vec<StructComponent>,
}

impl FunctionParameter {
    /// Creates a function parameter from a type argument
    pub fn from_type_arg(pat_type: &PatType) -> Result<Self, ABIError> {
        let arg_info = Self::parse_argument_info(pat_type)?;
        let (sol_type, data_location) = Self::resolve_solidity_type(&arg_info.ty)?;
        let internal_type = Self::resolve_internal_type(&arg_info.ty)?;
        let components = sol_type.get_components();

        Ok(Self {
            name: Some(arg_info.name.clone()),
            sol_type,
            internal_type,
            data_location,
            components,
            fn_arg: Some(arg_info),
        })
    }

    /// Creates a function parameter from a function argument, skipping self parameter
    pub fn from_fn_arg(arg: &FnArg) -> Option<Result<Self, ABIError>> {
        match arg {
            FnArg::Typed(pat_type) => Some(Self::from_type_arg(pat_type)),
            FnArg::Receiver(_) => None,
        }
    }

    /// Converts parameter back to function argument
    pub fn to_fn_arg(&self) -> Result<FnArg, ABIError> {
        let arg_info = self
            .fn_arg
            .as_ref()
            .ok_or_else(|| ABIError::Internal("Missing argument info".into()))?;

        let name = format_ident!("{}", arg_info.name);
        let ty = &arg_info.ty;

        let pat = if arg_info.is_mutable {
            quote! { mut #name }
        } else {
            quote! { #name }
        };

        syn::parse2(quote! { #pat: #ty }).map_err(|e| ABIError::Syntax(e.to_string()))
    }

    /// Creates a function parameter from a Type
    pub fn from_type(ty: &Type, name: Option<String>) -> Result<Self, ABIError> {
        let (sol_type, data_location) = Self::resolve_solidity_type(ty)?;
        let internal_type = Self::resolve_internal_type(ty)?;
        let components = sol_type.get_components();

        Ok(Self {
            name,
            sol_type,
            internal_type,
            data_location,
            components,
            fn_arg: None,
        })
    }

    pub fn from_return_type(return_type: &ReturnType) -> Result<Vec<Self>, ABIError> {
        match return_type {
            ReturnType::Default => Ok(vec![]),
            ReturnType::Type(_, ty) => match &**ty {
                Type::Tuple(tuple) => tuple
                    .elems
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| {
                        let name = format!("_{}", i);
                        let (sol_type, data_location) = Self::resolve_solidity_type(ty)?;
                        let internal_type = Self::resolve_internal_type(ty)?;
                        let components = sol_type.get_components();

                        Ok(Self {
                            name: Some(name.clone()),
                            sol_type,
                            internal_type,
                            components,
                            data_location,
                            fn_arg: Some(ArgumentInfo {
                                name,
                                ty: ty.clone(),
                                is_mutable: false,
                                pattern: None,
                            }),
                        })
                    })
                    .collect(),
                _ => {
                    let name = "_0".to_string();
                    let (sol_type, data_location) = Self::resolve_solidity_type(ty)?;
                    let internal_type = Self::resolve_internal_type(ty)?;
                    let components = sol_type.get_components();

                    Ok(vec![Self {
                        name: Some(name.clone()),
                        sol_type,
                        internal_type,
                        components,
                        data_location,
                        fn_arg: Some(ArgumentInfo {
                            name,
                            ty: (**ty).clone(),
                            is_mutable: false,
                            pattern: None,
                        }),
                    }])
                }
            },
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        match &self.components {
            Some(components) => json!({
                "name": self.name,
                "type": "tuple",
                "components": components.iter()
                    .map(FunctionParameter::to_json)
                    .collect::<Vec<_>>()
            }),
            None => json!({
                "name": self.name,
                "type": self.sol_type.to_string()
            }),
        }
    }

    fn load_struct_definition(struct_name: &str) -> Result<StructDefinition, ABIError> {
        let out_dir = env::var("OUT_DIR")
            .map_err(|_| ABIError::Config("OUT_DIR environment variable not found".into()))?;

        let file_path = PathBuf::from(out_dir)
            .join("solidity_abi")
            .join(format!("{}.json", struct_name));

        let content = fs::read_to_string(&file_path).map_err(|e| {
            ABIError::Config(format!("Failed to read struct definition file: {}", e))
        })?;

        serde_json::from_str(&content)
            .map_err(|e| ABIError::Config(format!("Failed to parse struct definition: {}", e)))
    }

    fn component_to_sol_type(component: &StructComponent) -> Result<(String, SolType), ABIError> {
        if !component.components.is_empty() {
            let fields = component
                .components
                .iter()
                .map(|c| {
                    let (name, sol_type) = Self::component_to_sol_type(c)?;
                    Ok((name, sol_type))
                })
                .collect::<Result<Vec<_>, ABIError>>()?;

            Ok((
                component.name.clone(),
                SolType::Struct {
                    name: component.type_name.clone(),
                    fields,
                },
            ))
        } else {
            let sol_type = match component.type_name.as_str() {
                "uint8" => SolType::Uint(8),
                "uint16" => SolType::Uint(16),
                "uint32" => SolType::Uint(32),
                "uint64" => SolType::Uint(64),
                "uint128" => SolType::Uint(128),
                "uint256" => SolType::Uint(256),
                "int8" => SolType::Int(8),
                "int16" => SolType::Int(16),
                "int32" => SolType::Int(32),
                "int64" => SolType::Int(64),
                "int128" => SolType::Int(128),
                "int256" => SolType::Int(256),
                "bool" => SolType::Bool,
                "address" => SolType::Address,
                "string" => SolType::String,
                "bytes" => SolType::Bytes,
                unknown => {
                    return Err(ABIError::UnsupportedType(format!(
                        "Unknown type in struct definition: {}",
                        unknown
                    )))
                }
            };
            Ok((component.name.clone(), sol_type))
        }
    }

    /// Generates tokens for parameter declaration in function body
    ///
    /// # Examples:
    /// ```ignore
    /// // For parameter declarations:
    /// value: u256        -> value
    /// data: &[u8]        -> data
    /// amount: &mut u256  -> mut amount
    /// ```
    pub fn to_declaration_tokens(&self) -> TokenStream2 {
        let arg_info = self
            .fn_arg
            .as_ref()
            .expect("Parameter must have argument info");

        let ident = format_ident!("{}", arg_info.name);
        if arg_info.is_mutable {
            quote! { mut #ident }
        } else {
            quote! { #ident }
        }
    }

    /// Generates tokens for using parameter as function argument
    ///
    /// # Examples:
    /// ```ignore
    /// // For function calls:
    /// value: u256        -> value
    /// data: &[u8]        -> &data
    /// amount: &mut u256  -> &mut amount
    /// ```
    pub fn to_argument_tokens(&self) -> TokenStream2 {
        let arg_info = self
            .fn_arg
            .as_ref()
            .expect("Parameter must have argument info");

        let ident = format_ident!("{}", arg_info.name);

        match &arg_info.ty {
            Type::Reference(ty_ref) => {
                if ty_ref.mutability.is_some() {
                    quote! { &mut #ident }
                } else {
                    quote! { &#ident }
                }
            }
            _ => quote! { #ident },
        }
    }

    /// Gets the Rust type of the parameter
    ///
    /// # Returns
    /// * `Option<&Type>` - Returns the Rust type if available, None if no type information exists
    pub fn get_rust_type(&self) -> Option<&Type> {
        self.fn_arg.as_ref().map(|arg| &arg.ty)
    }

    /// Extracts argument information from a PatType
    fn parse_argument_info(pat_type: &PatType) -> Result<ArgumentInfo, ABIError> {
        match &*pat_type.pat {
            Pat::Ident(PatIdent {
                ident, mutability, ..
            }) => Ok(ArgumentInfo {
                name: ident.to_string(),
                ty: (*pat_type.ty).clone(),
                is_mutable: mutability.is_some(),
                pattern: Some((*pat_type.pat).clone()),
            }),
            _ => Err(ABIError::UnsupportedPattern("Complex pattern".into())),
        }
    }

    /// Resolves internal type name for custom types
    fn resolve_internal_type(ty: &Type) -> Result<String, ABIError> {
        let (sol_type, _) = Self::resolve_solidity_type(ty)?;
        match &sol_type {
            SolType::Struct { name, .. } => Ok(format!("struct {}", name)),
            _ => Ok(sol_type.to_string()),
        }
    }

    fn resolve_struct_fields(struct_name: &str) -> Result<Vec<(String, SolType)>, ABIError> {
        let definition = Self::load_struct_definition(struct_name)?;

        definition
            .components
            .iter()
            .map(Self::component_to_sol_type)
            .collect()
    }

    /// Resolves Solidity type and data location from Rust type
    fn resolve_solidity_type(ty: &Type) -> Result<(SolType, Option<DataLocation>), ABIError> {
        match ty {
            Type::Reference(type_ref) => {
                let (inner_type, _) = Self::resolve_solidity_type(&type_ref.elem)?;
                Ok((inner_type, Some(DataLocation::Memory)))
            }
            Type::Array(array_type) => Self::resolve_array_type(array_type),
            Type::Tuple(tuple_type) => Self::resolve_tuple_type(tuple_type),
            Type::Path(type_path) => Self::resolve_path_type(type_path),
            Type::Slice(slice_type) => Self::resolve_slice_type(slice_type),
            _ => Err(ABIError::UnsupportedType(format!(
                "Unsupported type: {:?}",
                ty
            ))),
        }
    }

    fn resolve_array_type(ty: &TypeArray) -> Result<(SolType, Option<DataLocation>), ABIError> {
        let len = match &ty.len {
            syn::Expr::Lit(expr_lit) => {
                if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                    lit_int
                        .base10_parse::<usize>()
                        .map_err(|_| ABIError::UnsupportedType("Invalid array length".into()))?
                } else {
                    return Err(ABIError::UnsupportedType(
                        "Invalid array length literal".into(),
                    ));
                }
            }
            _ => return Err(ABIError::UnsupportedType("Non-literal array length".into())),
        };

        let (elem_type, _) = Self::resolve_solidity_type(&ty.elem)?;
        Ok((
            SolType::FixedArray(Box::new(elem_type), len),
            Some(DataLocation::Memory),
        ))
    }

    fn resolve_tuple_type(ty: &TypeTuple) -> Result<(SolType, Option<DataLocation>), ABIError> {
        let mut types = Vec::new();
        for elem in ty.elems.iter() {
            let (elem_type, _) = Self::resolve_solidity_type(elem)?;
            types.push(elem_type);
        }
        Ok((SolType::Tuple(types), Some(DataLocation::Memory)))
    }

    fn resolve_slice_type(ty: &TypeSlice) -> Result<(SolType, Option<DataLocation>), ABIError> {
        let (elem_type, _) = Self::resolve_solidity_type(&ty.elem)?;
        Ok((
            SolType::Array(Box::new(elem_type)),
            Some(DataLocation::Memory),
        ))
    }

    fn resolve_path_type(
        type_path: &TypePath,
    ) -> Result<(SolType, Option<DataLocation>), ABIError> {
        let last_segment = type_path
            .path
            .segments
            .last()
            .ok_or_else(|| ABIError::UnsupportedType("Empty type path".into()))?;

        let type_name = last_segment.ident.to_string();

        if UNSUPPORTED_TYPES.contains(&type_name.as_str()) {
            return Err(ABIError::UnsupportedType(format!(
                "Unsupported type: {}",
                type_name
            )));
        }

        match type_name.as_str() {
            // Unsigned integers
            "u8" => Ok((SolType::Uint(8), None)),
            "u16" => Ok((SolType::Uint(16), None)),
            "u32" => Ok((SolType::Uint(32), None)),
            "u64" => Ok((SolType::Uint(64), None)),
            "u128" => Ok((SolType::Uint(128), None)),
            "u256" | "U256" => Ok((SolType::Uint(256), None)),

            // Signed integers
            "i8" => Ok((SolType::Int(8), None)),
            "i16" => Ok((SolType::Int(16), None)),
            "i32" => Ok((SolType::Int(32), None)),
            "i64" => Ok((SolType::Int(64), None)),
            "i128" => Ok((SolType::Int(128), None)),
            "i256" | "I256" => Ok((SolType::Int(256), None)),

            // Other primitive types
            "bool" => Ok((SolType::Bool, None)),
            "Address" => Ok((SolType::Address, None)),
            "String" | "str" => Ok((SolType::String, Some(DataLocation::Memory))),
            "Bytes" => Ok((SolType::Bytes, Some(DataLocation::Memory))),

            // Complex types
            "Vec" => Self::resolve_vec_type(last_segment),
            "FixedBytes" => Self::resolve_fixed_bytes(&type_name, &last_segment.arguments),

            // Custom types (assumed to be structs)
            _ if Self::is_struct_type(type_path) => {
                let fields = Self::resolve_struct_fields(&type_name)?;

                Ok((
                    SolType::Struct {
                        name: type_name,
                        fields,
                    },
                    Some(DataLocation::Memory),
                ))
            }

            unknown => Err(ABIError::UnsupportedType(format!(
                "Unknown type: {}",
                unknown
            ))),
        }
    }

    fn resolve_vec_type(
        segment: &PathSegment,
    ) -> Result<(SolType, Option<DataLocation>), ABIError> {
        if let PathArguments::AngleBracketed(args) = &segment.arguments {
            if let Some(GenericArgument::Type(elem_type)) = args.args.first() {
                let (inner_type, _) = Self::resolve_solidity_type(elem_type)?;
                Ok((
                    SolType::Array(Box::new(inner_type)),
                    Some(DataLocation::Memory),
                ))
            } else {
                Err(ABIError::UnsupportedType(
                    "Invalid Vec type argument".into(),
                ))
            }
        } else {
            Err(ABIError::UnsupportedType("Invalid Vec type".into()))
        }
    }

    fn resolve_fixed_bytes(
        type_name: &str,
        args: &PathArguments,
    ) -> Result<(SolType, Option<DataLocation>), ABIError> {
        if let PathArguments::AngleBracketed(angle_bracketed_args) = args {
            if let Some(GenericArgument::Const(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(lit_int),
                ..
            }))) = angle_bracketed_args.args.first()
            {
                if let Ok(size) = lit_int.base10_parse::<usize>() {
                    if size > 0 && size <= 32 {
                        return Ok((SolType::FixedBytes(size), None));
                    }
                }
            }
        }
        Err(ABIError::UnsupportedType(format!(
            "{} requires a size parameter between 1 and 32",
            type_name
        )))
    }

    fn is_struct_type(type_path: &TypePath) -> bool {
        if let Some(segment) = type_path.path.segments.last() {
            !matches!(
                segment.ident.to_string().as_str(),
                "u8" | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "U256"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "I256"
                    | "bool"
                    | "String"
                    | "str"
                    | "Address"
                    | "Bytes"
                    | "Vec"
                    | "FixedBytes"
            )
        } else {
            false
        }
    }
}

impl SolType {
    fn abi_type(&self) -> String {
        match self {
            SolType::Uint(bits) => format!("uint{}", bits),
            SolType::Int(bits) => format!("int{}", bits),
            SolType::Address => "address".to_string(),
            SolType::Bool => "bool".to_string(),
            SolType::String => "string".to_string(),
            SolType::Bytes => "bytes".to_string(),
            SolType::FixedBytes(size) => format!("bytes{}", size),
            SolType::Array(inner) => format!("{}[]", inner.abi_type()),
            SolType::FixedArray(inner, size) => format!("{}[{}]", inner.abi_type(), size),
            SolType::Tuple(_) => "tuple".to_string(),
            SolType::Struct { .. } => "tuple".to_string(),
        }
    }

    pub fn get_components(&self) -> Option<Vec<FunctionParameter>> {
        match self {
            SolType::Struct { fields, .. } => Some(
                fields
                    .iter()
                    .map(|(name, field_type)| FunctionParameter {
                        name: Some(name.clone()),
                        sol_type: field_type.clone(),
                        internal_type: field_type.to_string(),
                        data_location: None,
                        components: field_type.get_components(),
                        fn_arg: None,
                    })
                    .collect(),
            ),

            SolType::Tuple(types) => Some(
                types
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| FunctionParameter {
                        name: Some(format!("_{}", i)),
                        sol_type: ty.clone(),
                        internal_type: ty.to_string(),
                        data_location: None,
                        components: ty.get_components(),
                        fn_arg: None,
                    })
                    .collect(),
            ),

            SolType::Array(inner) | SolType::FixedArray(inner, _) => match &**inner {
                SolType::Struct { .. } | SolType::Tuple(_) => inner.get_components(),
                _ => None,
            },

            _ => None,
        }
    }
}

impl Display for SolType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            // Primitive types
            SolType::Uint(bits) => write!(f, "uint{}", bits),
            SolType::Int(bits) => write!(f, "int{}", bits),
            SolType::Address => write!(f, "address"),
            SolType::Bool => write!(f, "bool"),
            SolType::String => write!(f, "string"),
            SolType::Bytes => write!(f, "bytes"),
            SolType::FixedBytes(size) => write!(f, "bytes{}", size),

            // Container types
            SolType::Array(inner) => write!(f, "{}[]", inner),
            SolType::FixedArray(inner, size) => write!(f, "{}[{}]", inner, size),

            SolType::Tuple(types) => {
                write!(f, "(")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, ")")
            }

            SolType::Struct { .. } => write!(f, "tuple"),
        }
    }
}

impl Serialize for SolType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.abi_type())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;
    use std::path::Path;
    use syn::parse_quote;
    use tempfile::TempDir;

    fn assert_type(ty: &Type, expected: &str) {
        let (sol_type, _) =
            FunctionParameter::resolve_solidity_type(ty).expect("Failed to resolve Solidity type");

        assert_eq!(
            sol_type.to_string(),
            expected,
            "Type conversion failed for {:?}",
            ty
        );
    }

    #[test]
    fn test_rust_to_sol_type_conversion() {
        let cases = [
            // Numeric types
            (parse_quote!(u8), "uint8"),
            (parse_quote!(u16), "uint16"),
            (parse_quote!(u32), "uint32"),
            (parse_quote!(u64), "uint64"),
            (parse_quote!(u128), "uint128"),
            (parse_quote!(U256), "uint256"),
            // Basic types
            (parse_quote!(bool), "bool"),
            (parse_quote!(Address), "address"),
            (parse_quote!(String), "string"),
            // Arrays
            (parse_quote!(Vec<u8>), "uint8[]"),
            (parse_quote!([u8; 32]), "uint8[32]"),
            (parse_quote!([u8; 20]), "uint8[20]"),
            // Tuples
            (parse_quote!((u8, bool)), "(uint8,bool)"),
            // References
            (parse_quote!(&String), "string"),
            (parse_quote!(&[u8]), "uint8[]"),
            // Bytes
            (parse_quote!(Bytes), "bytes"),
            (parse_quote!(FixedBytes<32>), "bytes32"),
            (parse_quote!(FixedBytes<20>), "bytes20"),
        ];

        for (ref rust_type, expected_sol_type) in cases {
            assert_type(rust_type, expected_sol_type);
        }
    }

    #[test]
    fn test_parameter_abi_json() {
        let arg: FnArg = parse_quote!(amount: u256);
        let param = FunctionParameter::from_fn_arg(&arg)
            .expect("Should return Some for regular parameter")
            .expect("Should successfully parse parameter");

        let json = serde_json::to_value(&param).unwrap();
        assert_eq!(
            json,
            json!({
                "name": "amount",
                "type": "uint256",
                "internalType": "uint256",
            })
        );

        let arg: FnArg = parse_quote!(data: String);
        let param = FunctionParameter::from_fn_arg(&arg)
            .expect("Should return Some for regular parameter")
            .expect("Should successfully parse parameter");

        let json = serde_json::to_value(&param).unwrap();
        assert_eq!(
            json,
            json!({
                "name": "data",
                "type": "string",
                "internalType": "string",
            })
        );

        let arg: FnArg = parse_quote!(self);
        assert!(
            FunctionParameter::from_fn_arg(&arg).is_none(),
            "Should return None for self parameter"
        );
    }

    #[test]
    fn test_complex_types_abi_json() {
        fn test_struct_type() -> SolType {
            SolType::Struct {
                name: "MyStruct".to_string(),
                fields: vec![
                    ("value".to_string(), SolType::Uint(256)),
                    ("active".to_string(), SolType::Bool),
                ],
            }
        }

        fn test_tuple_type() -> SolType {
            SolType::Tuple(vec![SolType::Uint(256), SolType::Bool, SolType::Address])
        }

        let struct_param = FunctionParameter {
            name: Some("myStruct".to_string()),
            sol_type: test_struct_type(),
            internal_type: "struct MyStruct".to_string(),
            components: test_struct_type().get_components(),
            data_location: Some(DataLocation::Memory),
            fn_arg: None,
        };

        let array_struct_param = FunctionParameter {
            name: Some("structs".to_string()),
            sol_type: SolType::Array(Box::new(test_struct_type())),
            internal_type: "struct MyStruct[]".to_string(),
            components: test_struct_type().get_components(),
            data_location: Some(DataLocation::Memory),
            fn_arg: None,
        };

        let tuple_param = FunctionParameter {
            name: Some("tuple".to_string()),
            sol_type: test_tuple_type(),
            internal_type: "tuple".to_string(),
            components: test_tuple_type().get_components(),
            data_location: None,
            fn_arg: None,
        };

        let array_tuple_param = FunctionParameter {
            name: Some("tuples".to_string()),
            sol_type: SolType::Array(Box::new(test_tuple_type())),
            internal_type: "tuple[]".to_string(),
            components: test_tuple_type().get_components(),
            data_location: Some(DataLocation::Memory),
            fn_arg: None,
        };

        assert_eq!(
            serde_json::to_value(&struct_param).unwrap(),
            json!({
                "name": "myStruct",
                "type": "tuple",
                "internalType": "struct MyStruct",
                "components": [
                    {
                        "name": "value",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "active",
                        "type": "bool",
                        "internalType": "bool"
                    }
                ]
            })
        );

        assert_eq!(
            serde_json::to_value(&array_struct_param).unwrap(),
            json!({
                "name": "structs",
                "type": "tuple[]",
                "internalType": "struct MyStruct[]",
                "components": [
                    {
                        "name": "value",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "active",
                        "type": "bool",
                        "internalType": "bool"
                    }
                ]
            })
        );

        assert_eq!(
            serde_json::to_value(&tuple_param).unwrap(),
            json!({
                "name": "tuple",
                "type": "tuple",
                "internalType": "tuple",
                "components": [
                    {
                        "name": "_0",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "_1",
                        "type": "bool",
                        "internalType": "bool"
                    },
                    {
                        "name": "_2",
                        "type": "address",
                        "internalType": "address"
                    }
                ]
            })
        );

        assert_eq!(
            serde_json::to_value(&array_tuple_param).unwrap(),
            json!({
                "name": "tuples",
                "type": "tuple[]",
                "internalType": "tuple[]",
                "components": [
                    {
                        "name": "_0",
                        "type": "uint256",
                        "internalType": "uint256"
                    },
                    {
                        "name": "_1",
                        "type": "bool",
                        "internalType": "bool"
                    },
                    {
                        "name": "_2",
                        "type": "address",
                        "internalType": "address"
                    }
                ]
            })
        );
    }

    #[test]
    fn test_from_type() {
        // Test simple type
        let ty: Type = parse_quote!(u256);
        let param = FunctionParameter::from_type(&ty, Some("amount".to_string())).unwrap();
        assert_eq!(param.name, Some("amount".to_string()));
        assert_eq!(param.internal_type, "uint256");

        // Test tuple type
        let ty: Type = parse_quote!((u256, bool));
        let param = FunctionParameter::from_type(&ty, Some("pair".to_string())).unwrap();
        assert_eq!(param.name, Some("pair".to_string()));
        assert_eq!(param.internal_type, "(uint256,bool)");
    }

    #[test]
    fn test_from_return_type() {
        // Test single return type
        let return_type: ReturnType = parse_quote!(-> u256);
        let params = FunctionParameter::from_return_type(&return_type).unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, Some("_0".to_string()));
        assert_eq!(params[0].internal_type, "uint256");

        // Test tuple return type
        let return_type: ReturnType = parse_quote!(-> (u256, bool));
        let params = FunctionParameter::from_return_type(&return_type).unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, Some("_0".to_string()));
        assert_eq!(params[0].internal_type, "uint256");
        assert_eq!(params[1].name, Some("_1".to_string()));
        assert_eq!(params[1].internal_type, "bool");
    }

    #[test]
    #[serial]
    fn test_struct_loading() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("OUT_DIR", temp_dir.path());

        let abi_dir = temp_dir.path().join("solidity_abi");
        std::fs::create_dir_all(&abi_dir).unwrap();

        let test_json = json!({
            "type": "tuple",
            "components": [
                {"name": "x", "type": "uint64"},
                {"name": "y", "type": "uint256"}
            ]
        });

        let file_path = abi_dir.join("Point.json");
        std::fs::write(
            &file_path,
            serde_json::to_string_pretty(&test_json).unwrap(),
        )
        .unwrap();

        let fields = FunctionParameter::resolve_struct_fields("Point").unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "x");
        assert_eq!(fields[1].0, "y");
    }

    fn setup_test_files(temp_dir: &Path) -> std::io::Result<()> {
        let abi_dir = temp_dir.join("solidity_abi");
        fs::create_dir_all(&abi_dir)?;

        let point_json = json!({
            "type": "tuple",
            "components": [
                {"name": "x", "type": "uint64"},
                {"name": "y", "type": "uint256"}
            ]
        });

        let complex_point_json = json!({
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
        });

        fs::write(
            abi_dir.join("Point.json"),
            serde_json::to_string_pretty(&point_json)?,
        )?;

        fs::write(
            abi_dir.join("ComplexPoint.json"),
            serde_json::to_string_pretty(&complex_point_json)?,
        )?;

        Ok(())
    }

    #[test]
    #[serial]
    fn test_nested_struct_loading() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("OUT_DIR", temp_dir.path());

        setup_test_files(temp_dir.path()).unwrap();

        let fields = FunctionParameter::resolve_struct_fields("ComplexPoint").unwrap();

        assert_eq!(fields.len(), 3, "ComplexPoint should have 3 fields");

        let (point_name, point_type) = &fields[0];
        assert_eq!(point_name, "point", "First field should be named 'point'");

        match point_type {
            SolType::Struct { name, fields } => {
                assert_eq!(name, "tuple", "Nested struct should have type 'tuple'");
                assert_eq!(fields.len(), 2, "Point should have 2 fields");

                let (x_name, x_type) = &fields[0];
                let (y_name, y_type) = &fields[1];

                assert_eq!(x_name, "x", "First nested field should be named 'x'");
                assert!(matches!(x_type, SolType::Uint(64)), "x should be uint64");

                assert_eq!(y_name, "y", "Second nested field should be named 'y'");
                assert!(matches!(y_type, SolType::Uint(256)), "y should be uint256");
            }
            _ => panic!("Expected point to be a struct type"),
        }

        let (desc_name, desc_type) = &fields[1];
        assert_eq!(
            desc_name, "description",
            "Second field should be named 'description'"
        );
        assert!(
            matches!(desc_type, SolType::String),
            "description should be string type"
        );

        let (active_name, active_type) = &fields[2];
        assert_eq!(
            active_name, "active",
            "Third field should be named 'active'"
        );
        assert!(
            matches!(active_type, SolType::Bool),
            "active should be bool type"
        );

        let param = FunctionParameter::from_type(
            &syn::parse_quote!(ComplexPoint),
            Some("complex_point".to_string()),
        )
        .unwrap();

        let json = serde_json::to_value(&param).unwrap();

        assert_eq!(
            json,
            json!({
                "name": "complex_point",
                "type": "tuple",
                "internalType": "struct ComplexPoint",
                "components": [
                    {
                        "name": "point",
                        "type": "tuple",
                        "internalType": "tuple",
                        "components": [
                            {
                                "name": "x",
                                "type": "uint64",
                                "internalType": "uint64"
                            },
                            {
                                "name": "y",
                                "type": "uint256",
                                "internalType": "uint256"
                            }
                        ]
                    },
                    {
                        "name": "description",
                        "type": "string",
                        "internalType": "string"
                    },
                    {
                        "name": "active",
                        "type": "bool",
                        "internalType": "bool"
                    }
                ]
            })
        );
    }
}
