use std::fmt::{self, Display, Formatter};

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

impl SolType {
    #[must_use]
    pub fn abi_type(&self) -> String {
        self.to_string()
    }

    #[must_use]
    pub fn abi_type_internal(&self) -> String {
        match self {
            Self::Struct { name, .. } => format!("struct {name}"),
            Self::Array(inner) => match &**inner {
                Self::Struct { name, .. } => format!("struct {name}[]"),
                _ => self.to_string(),
            },
            Self::FixedArray(inner, size) => match &**inner {
                Self::Struct { name, .. } => format!("struct {name}[{size}]"),
                _ => self.to_string(),
            },
            _ => self.to_string(),
        }
    }
}

impl Display for SolType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uint(bits) => write!(f, "uint{bits}"),
            Self::Int(bits) => write!(f, "int{bits}"),
            Self::Address => write!(f, "address"),
            Self::Bool => write!(f, "bool"),
            Self::String => write!(f, "string"),
            Self::Bytes => write!(f, "bytes"),
            Self::FixedBytes(size) => write!(f, "bytes{size}"),
            Self::Array(inner) => write!(f, "{inner}[]"),
            Self::FixedArray(inner, size) => write!(f, "{inner}[{size}]"),
            Self::Tuple(types) => {
                write!(f, "(")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{ty}")?;
                }
                write!(f, ")")
            }
            Self::Struct { .. } => write!(f, "tuple"),
        }
    }
}
