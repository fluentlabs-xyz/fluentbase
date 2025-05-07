use crate::{
    attr::{function_id::create_function_id_mismatch_error, FunctionIDAttribute},
    signature::ParsedSignature,
};
use syn::{spanned::Spanned, Attribute, ImplItemFn, Signature, TraitItemFn};

/// Trait defining common behavior for trait and impl methods
pub trait MethodLike: Sized {
    fn sig(&self) -> &Signature;
    fn attrs(&self) -> &Vec<Attribute>;

    fn function_id_attr(&self) -> syn::Result<Option<FunctionIDAttribute>> {
        self.attrs()
            .iter()
            .find(|a| a.path().is_ident("function_id"))
            .map(|attr| attr.parse_args::<FunctionIDAttribute>())
            .transpose()
    }
}

// Implement MethodLike for TraitItemFn
impl MethodLike for TraitItemFn {
    fn sig(&self) -> &Signature {
        &self.sig
    }

    fn attrs(&self) -> &Vec<Attribute> {
        &self.attrs
    }
}

// Implement MethodLike for ImplItemFn
impl MethodLike for ImplItemFn {
    fn sig(&self) -> &Signature {
        &self.sig
    }

    fn attrs(&self) -> &Vec<Attribute> {
        &self.attrs
    }
}

/// Represents a parsed method in a contract
#[derive(Debug, Clone)]
pub struct ParsedMethod<T: MethodLike> {
    /// Function ID calculated from the method signature
    function_id: crate::attr::function_id::FunctionID,

    /// Parsed signature of the method
    sig: ParsedSignature,

    /// Inner function implementation
    inner: T,
}

impl<T: MethodLike> ParsedMethod<T> {
    /// Creates a new ParsedMethod with given inner implementation
    pub fn new(inner: T) -> syn::Result<Self> {
        let sig = ParsedSignature::new(inner.sig().clone());
        let function_id = sig.function_abi()?.function_id()?;

        if let Some(attr) = inner.function_id_attr()? {
            let function_id_attr = attr.function_id_bytes()?;

            if attr.is_validation_enabled() {
                if function_id_attr != function_id {
                    return Err(create_function_id_mismatch_error(
                        inner.sig().span(),
                        &function_id,
                        &function_id_attr,
                        sig.function_abi()?.signature()?,
                    ));
                }
            } else {
                // If validation is disabled, use the function ID from the attribute
                return Ok(Self {
                    function_id: function_id_attr,
                    sig,
                    inner,
                });
            }
        }

        Ok(Self {
            function_id,
            sig,
            inner,
        })
    }

    /// Creates a new ParsedMethod from a reference
    pub fn from_ref(inner: &T) -> syn::Result<Self>
    where
        T: Clone,
    {
        Self::new(inner.clone())
    }

    /// Returns the function ID
    pub fn function_id(&self) -> &crate::attr::function_id::FunctionID {
        &self.function_id
    }

    /// Returns a reference to the inner signature
    pub fn sig(&self) -> &Signature {
        self.inner.sig()
    }

    /// Returns the function's parsed signature
    pub fn parsed_signature(&self) -> &ParsedSignature {
        &self.sig
    }

    /// Returns a reference to the inner implementation
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

// Implement From<ImplItemFn> for ParsedMethod<ImplItemFn>
impl From<ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: ImplItemFn) -> Self {
        Self::new(function).expect("Failed to parse method")
    }
}

// Also implement From for reference to ImplItemFn to match our visitor's usage pattern
impl From<&ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: &ImplItemFn) -> Self {
        Self::from_ref(function).expect("Failed to parse method from reference")
    }
}
