use crate::{
    attr::{function_id::FunctionID, FunctionIDAttribute},
    signature::ParsedSignature,
};
use proc_macro2::Span;
use proc_macro_error::{abort, abort_call_site};
use std::collections::HashSet;
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
    Attribute,
    Error,
    FnArg,
    ImplItemFn,
    ReturnType,
    Signature,
    TraitItemFn,
    Visibility,
};

// Constants for special method names
const FALLBACK_METHOD: &str = "fallback";
const DEPLOY_METHOD: &str = "deploy";

/// Trait defining common behavior for trait and impl methods
pub trait MethodLike: Sized {
    fn sig(&self) -> &Signature;
    fn attrs(&self) -> &Vec<Attribute>;

    /// Extracts function ID attribute if present
    fn function_id_attr(&self) -> syn::Result<Option<(FunctionIDAttribute, Span)>> {
        self.attrs()
            .iter()
            .find(|a| a.path().is_ident("function_id"))
            .map(|attr| {
                let content_span = attr.meta.require_list()?.tokens.span();
                attr.parse_args::<FunctionIDAttribute>()
                    .map(|parsed| (parsed, content_span))
            })
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
    function_id: FunctionID,
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

        // Handle custom function ID if defined via attribute
        if let Some((attr, attr_span)) = inner.function_id_attr()? {
            let function_id_attr = attr.function_id_bytes()?;

            if attr.is_validation_enabled() && function_id_attr != function_id {
                abort!(
                    attr_span,
                    "Function ID mismatch: Expected 0x{} for '{}', but got 0x{}",
                    hex::encode(function_id),
                    sig.function_abi()?.signature()?,
                    hex::encode(function_id_attr);
                    note = "You're seeing this error because you have validation enabled (validate(true))";
                    help = "To fix this, you can either:";
                    help = "1. Use the expected function ID: #[function_id(\"{}\")]", sig.function_abi()?.signature()?;
                    help = "2. Remove the validate parameter entirely: #[function_id(\"{}\")]", attr.signature().unwrap_or_else(|| "your_signature".to_string());
                    help = "3. Or explicitly disable validation: #[function_id(\"{}\", validate(false))]", attr.signature().unwrap_or_else(|| "your_signature".to_string())
                );
            } else if !attr.is_validation_enabled() {
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
    pub fn function_id(&self) -> FunctionID {
        self.function_id
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

/// Collector for gathering methods from trait or impl blocks
pub struct MethodCollector<T: MethodLike> {
    /// Collected methods
    pub methods: Vec<ParsedMethod<T>>,
    /// Source span for error reporting
    pub span: Span,
    /// Collection of errors encountered during parsing
    pub errors: Vec<Error>,
    /// Set of method selectors to detect collisions
    pub selectors: HashSet<FunctionID>,
    /// Whether this is a trait implementation (for router)
    pub is_trait_impl: bool,
}

impl<T: MethodLike> MethodCollector<T> {
    /// Creates a new method collector for trait methods
    pub fn new(span: Span) -> Self {
        Self {
            methods: Vec::new(),
            span,
            errors: Vec::new(),
            selectors: HashSet::new(),
            is_trait_impl: false,
        }
    }

    /// Creates a new method collector for impl methods with trait flag
    pub fn new_for_impl(span: Span, is_trait_impl: bool) -> Self {
        Self {
            methods: Vec::new(),
            span,
            errors: Vec::new(),
            selectors: HashSet::new(),
            is_trait_impl,
        }
    }

    /// Validates that there are no function selector collisions
    pub fn validate_selectors(&self) -> Result<(), Error> {
        if self.selectors.len() == self.methods.len() {
            return Ok(());
        }

        // Find the first method with a collision
        for method in &self.methods {
            let selector = method.function_id();
            let count = self
                .methods
                .iter()
                .filter(|m| m.function_id() == selector)
                .count();

            if count > 1 {
                return Err(Error::new(
                    method.parsed_signature().span(),
                    format!(
                        "Function selector collision detected for '{}'. Selector: {:02x}{:02x}{:02x}{:02x}",
                        method.parsed_signature().rust_name(),
                        selector[0], selector[1], selector[2], selector[3]
                    ),
                ));
            }
        }

        Ok(())
    }

    /// Checks if any errors were encountered during method collection
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Adds a method to the collection after checking for selector collision
    pub fn add_method(&mut self, method: ParsedMethod<T>) {
        let selector = method.function_id();
        if !self.selectors.insert(selector) {
            self.add_error(
                method.parsed_signature().span(),
                format!(
                    "Function selector collision detected for '{}'. Selector: {:02x}{:02x}{:02x}{:02x}",
                    method.parsed_signature().rust_name(),
                    selector[0], selector[1], selector[2], selector[3]
                ),
            );
        } else {
            self.methods.push(method);
        }
    }

    /// Adds an error to the collection
    pub fn add_error(&mut self, span: Span, message: String) {
        self.errors.push(Error::new(span, message));
    }

    /// Validates the signature of the fallback method
    ///
    /// A valid fallback method:
    /// - Takes only a single parameter (`self`)
    /// - Has no return type
    fn validate_fallback_signature(&self, sig: &Signature) -> bool {
        // Check if there's exactly 1 input and it's a receiver
        let has_only_self =
            sig.inputs.len() == 1 && matches!(sig.inputs.first(), Some(FnArg::Receiver(_)));

        let has_no_return = matches!(sig.output, ReturnType::Default);

        has_only_self && has_no_return
    }

    /// Handles fallback method validation
    fn handle_fallback_method(&mut self, method_sig: &Signature, span: Span) {
        if !self.validate_fallback_signature(method_sig) {
            self.add_error(
                span,
                format!("{} method must have signature 'fn {}(&self)' with no parameters and no return value",
                        FALLBACK_METHOD, FALLBACK_METHOD),
            );
        }
    }

    /// Handles regular method parsing and validation
    fn handle_regular_method(&mut self, method: &T)
    where
        T: Clone,
    {
        match ParsedMethod::from_ref(method) {
            Ok(parsed_method) => {
                self.add_method(parsed_method);
            }
            Err(err) => {
                self.add_error(
                    method.sig().span(),
                    format!("Failed to parse method: {}", err),
                );
            }
        }
    }
}

// Implementation for MethodCollector<TraitItemFn>
impl Visit<'_> for MethodCollector<TraitItemFn> {
    fn visit_trait_item_fn(&mut self, method: &TraitItemFn) {
        match method.sig.ident.to_string().as_str() {
            FALLBACK_METHOD => self.handle_fallback_method(&method.sig, method.sig.span()),
            DEPLOY_METHOD => {} // Skip deploy without validating its signature
            _ => self.handle_regular_method(method),
        }

        visit::visit_trait_item_fn(self, method);
    }
}

// Implementation for MethodCollector<ImplItemFn>
impl Visit<'_> for MethodCollector<ImplItemFn> {
    fn visit_impl_item_fn(&mut self, method: &ImplItemFn) {
        match method.sig.ident.to_string().as_str() {
            FALLBACK_METHOD => self.handle_fallback_method(&method.sig, method.sig.span()),
            DEPLOY_METHOD => {} // Skip deploy without validating
            _ => {
                let is_public = self.is_trait_impl || matches!(method.vis, Visibility::Public(_));
                if is_public {
                    self.handle_regular_method(method);
                }
            }
        }

        visit::visit_impl_item_fn(self, method);
    }
}

// Implement From for ImplItemFn
impl From<ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: ImplItemFn) -> Self {
        match Self::new(function) {
            Ok(method) => method,
            Err(err) => abort_call_site!("Failed to parse method: {}", err),
        }
    }
}

// Implement From for reference to ImplItemFn
impl From<&ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: &ImplItemFn) -> Self {
        match Self::from_ref(function) {
            Ok(method) => method,
            Err(err) => abort_call_site!("Failed to parse method from reference: {}", err),
        }
    }
}

// Implement From for TraitItemFn
impl From<TraitItemFn> for ParsedMethod<TraitItemFn> {
    fn from(function: TraitItemFn) -> Self {
        match Self::new(function) {
            Ok(method) => method,
            Err(err) => abort_call_site!("Failed to parse method: {}", err),
        }
    }
}

// Implement From for reference to TraitItemFn
impl From<&TraitItemFn> for ParsedMethod<TraitItemFn> {
    fn from(function: &TraitItemFn) -> Self {
        match Self::from_ref(function) {
            Ok(method) => method,
            Err(err) => abort_call_site!("Failed to parse method from reference: {}", err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_validate_fallback_signature() {
        let collector = MethodCollector::<TraitItemFn>::new(Span::call_site());

        // Valid fallback signature
        let valid_sig: Signature = parse_quote! {
            fn fallback(&self)
        };
        assert!(collector.validate_fallback_signature(&valid_sig));

        // Invalid: with parameters
        let invalid_with_params: Signature = parse_quote! {
            fn fallback(&self, param: u32)
        };
        assert!(!collector.validate_fallback_signature(&invalid_with_params));

        // Invalid: with return type
        let invalid_with_return: Signature = parse_quote! {
            fn fallback(&self) -> u32
        };
        assert!(!collector.validate_fallback_signature(&invalid_with_return));

        // Invalid: without self
        let invalid_without_self: Signature = parse_quote! {
            fn fallback()
        };
        assert!(!collector.validate_fallback_signature(&invalid_without_self));
    }

    #[test]
    fn test_function_id_collision() {
        let mut collector = MethodCollector::<TraitItemFn>::new(Span::call_site());

        // Create two trait methods with the same function ID selector
        let trait_fn1: TraitItemFn = parse_quote! {
            #[function_id("transfer(address,uint256)", validate(false))]
            fn first_method(&self);
        };

        let trait_fn2: TraitItemFn = parse_quote! {
            #[function_id("transfer(address,uint256)", validate(false))]
            fn second_method(&self);
        };

        // Process the first method
        let parsed1 = ParsedMethod::from(trait_fn1);
        collector.add_method(parsed1);
        assert_eq!(collector.methods.len(), 1);
        assert_eq!(collector.errors.len(), 0);

        // Process the second method with the same function ID
        let parsed2 = ParsedMethod::from(trait_fn2);
        collector.add_method(parsed2);

        // Should detect collision
        assert_eq!(collector.methods.len(), 1); // Second method not added
        assert_eq!(collector.errors.len(), 1); // Error reported
        assert!(collector.has_errors());
    }

    #[test]
    fn test_from_ref_impl() {
        // Create a simple impl item function
        let impl_fn: ImplItemFn = parse_quote! {
            pub fn simple_function(&self) {}
        };

        // Test that from_ref works without cloning unnecessarily
        let result = ParsedMethod::from_ref(&impl_fn);
        assert!(result.is_ok());
    }
}
