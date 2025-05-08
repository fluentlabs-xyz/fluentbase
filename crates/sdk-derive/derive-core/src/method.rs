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
    ImplItemFn,
    ReturnType,
    Signature,
    TraitItemFn,
    Visibility,
};

/// Trait defining common behavior for trait and impl methods
pub trait MethodLike: Sized {
    fn sig(&self) -> &Signature;
    fn attrs(&self) -> &Vec<Attribute>;

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

        if let Some((attr, attr_span)) = inner.function_id_attr()? {
            let function_id_attr = attr.function_id_bytes()?;

            if attr.is_validation_enabled() {
                if function_id_attr != function_id {
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
    pub fn function_id(&self) -> &FunctionID {
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

    /// Validates that there are no function selector collisions among collected methods
    pub fn validate_selectors(&self) -> Result<(), Error> {
        if self.selectors.len() != self.methods.len() {
            // This is a safety check in case we missed a collision during collection
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
        }

        Ok(())
    }

    /// Checks if any errors were encountered during method collection
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Adds a method to the collection after checking for selector collision
    pub fn add_method(&mut self, method: ParsedMethod<T>) {
        let selector = method.function_id().clone();
        if !self.selectors.insert(selector) {
            self.errors.push(Error::new(
                method.parsed_signature().span(),
                format!(
                    "Function selector collision detected for '{}'. Selector: {:02x}{:02x}{:02x}{:02x}",
                    method.parsed_signature().rust_name(),
                    selector[0], selector[1], selector[2], selector[3]
                ),
            ));
        } else {
            self.methods.push(method);
        }
    }

    /// Adds an error to the collection
    pub fn add_error(&mut self, span: Span, message: String) {
        self.errors.push(Error::new(span, message));
    }

    /// Validates the signature of the fallback method
    fn validate_fallback_signature(&self, sig: &Signature) -> bool {
        // Fallback should only have self parameter and no return type
        let has_only_self = sig.inputs.len() == 1
            && sig.inputs.iter().next().map_or(false, |arg| match arg {
                syn::FnArg::Receiver(_) => true,
                _ => false,
            });

        let has_no_return = matches!(sig.output, ReturnType::Default);

        has_only_self && has_no_return
    }
}

// Implementation for MethodCollector<TraitItemFn>
impl Visit<'_> for MethodCollector<TraitItemFn> {
    fn visit_trait_item_fn(&mut self, method: &TraitItemFn) {
        // Handle reserved method names with validation
        if method.sig.ident == "fallback" {
            if !self.validate_fallback_signature(&method.sig) {
                self.add_error(
                    method.sig.span(),
                    "Fallback method must have signature 'fn fallback(&self)' with no parameters and no return value".to_string(),
                );
            }
            // Don't collect fallback even if valid
        } else if method.sig.ident == "deploy" {
            // Skip deploy without validating its signature - deploy can have any signature
        } else {
            // Process regular methods
            let method_owned = TraitItemFn {
                attrs: method.attrs.clone(),
                sig: method.sig.clone(),
                default: method.default.clone(),
                semi_token: method.semi_token,
            };

            // Try to parse the method
            match ParsedMethod::new(method_owned) {
                Ok(parsed_method) => {
                    self.add_method(parsed_method);
                }
                Err(err) => {
                    // Store the error with the method's span for accurate reporting
                    self.add_error(
                        method.sig.span(),
                        format!("Failed to parse method: {}", err),
                    );
                }
            }
        }

        // Continue visiting child nodes
        visit::visit_trait_item_fn(self, method);
    }
}

// Implementation for MethodCollector<ImplItemFn>
impl Visit<'_> for MethodCollector<ImplItemFn> {
    fn visit_impl_item_fn(&mut self, method: &ImplItemFn) {
        // Handle reserved method names with validation
        if method.sig.ident == "fallback" {
            if !self.validate_fallback_signature(&method.sig) {
                self.add_error(
                    method.sig.span(),
                    "Fallback method must have signature 'fn fallback(&self)' with no parameters and no return value".to_string(),
                );
            }
            // Don't collect fallback even if valid
        } else if method.sig.ident == "deploy" {
            // Skip deploy without validating its signature - deploy can have any signature
        } else {
            // Process regular methods
            let is_public = self.is_trait_impl || matches!(method.vis, Visibility::Public(_));

            if is_public {
                // Clone the method to create an owned value
                let method_owned = ImplItemFn {
                    attrs: method.attrs.clone(),
                    vis: method.vis.clone(),
                    defaultness: method.defaultness,
                    sig: method.sig.clone(),
                    block: method.block.clone(),
                };

                // Try to parse the method
                match ParsedMethod::new(method_owned) {
                    Ok(parsed_method) => {
                        self.add_method(parsed_method);
                    }
                    Err(err) => {
                        // Store the error with the method's span for accurate reporting
                        self.add_error(
                            method.sig.span(),
                            format!("Failed to parse method: {}", err),
                        );
                    }
                }
            }
        }

        // Continue visiting child nodes
        visit::visit_impl_item_fn(self, method);
    }
}

// Implement From for ImplItemFn
impl From<ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: ImplItemFn) -> Self {
        match Self::new(function) {
            Ok(method) => method,
            Err(err) => {
                abort_call_site!("Failed to parse method: {}", err);
            }
        }
    }
}

// Implement From for reference to ImplItemFn
impl From<&ImplItemFn> for ParsedMethod<ImplItemFn> {
    fn from(function: &ImplItemFn) -> Self {
        match Self::from_ref(function) {
            Ok(method) => method,
            Err(err) => {
                abort_call_site!("Failed to parse method from reference: {}", err);
            }
        }
    }
}

// Implement From for TraitItemFn
impl From<TraitItemFn> for ParsedMethod<TraitItemFn> {
    fn from(function: TraitItemFn) -> Self {
        match Self::new(function) {
            Ok(method) => method,
            Err(err) => {
                abort_call_site!("Failed to parse method: {}", err);
            }
        }
    }
}

// Implement From for reference to TraitItemFn
impl From<&TraitItemFn> for ParsedMethod<TraitItemFn> {
    fn from(function: &TraitItemFn) -> Self {
        match Self::from_ref(function) {
            Ok(method) => method,
            Err(err) => {
                abort_call_site!("Failed to parse method from reference: {}", err);
            }
        }
    }
}
