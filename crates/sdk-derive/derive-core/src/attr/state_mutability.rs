use crate::abi::function::StateMutability;
use syn::{spanned::Spanned, Attribute, FnArg, LitStr, Signature};

/// Name of the attribute carrying the Solidity state mutability of a method.
pub const STATE_MUTABILITY_ATTR: &str = "state_mutability";

/// All values accepted by the `#[state_mutability(...)]` attribute
const VALID_MUTABILITIES: &[&str] = &["pure", "view", "nonpayable", "payable"];

/// Extension helpers describing what a call to a method of this mutability is
/// allowed to do on the host.
pub trait StateMutabilityExt: Sized {
    /// Returns true when the callee must not be able to mutate state, i.e. the
    /// call has to be issued as a `STATICCALL`
    fn is_static(&self) -> bool;

    /// Returns true when native value may be attached to the call
    fn allows_value(&self) -> bool;

    /// Returns the canonical Solidity name of the mutability
    fn as_str(&self) -> &'static str;

    /// Parses the canonical Solidity name of a mutability
    fn from_str(value: &str) -> Option<Self>;
}

impl StateMutabilityExt for StateMutability {
    fn is_static(&self) -> bool {
        matches!(self, Self::Pure | Self::View)
    }

    fn allows_value(&self) -> bool {
        matches!(self, Self::Payable)
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::View => "view",
            Self::NonPayable => "nonpayable",
            Self::Payable => "payable",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "pure" => Some(Self::Pure),
            "view" => Some(Self::View),
            "nonpayable" => Some(Self::NonPayable),
            "payable" => Some(Self::Payable),
            _ => None,
        }
    }
}

/// Parses the `#[state_mutability("...")]` attribute of a method, falling back
/// to the mutability implied by its receiver.
///
/// The Solidity front-end (`derive_solidity_client`) emits the attribute
/// explicitly so `pure`/`view` and `nonpayable`/`payable` survive code
/// generation. Hand-written traits only carry the receiver, so `&self` is read
/// as `view` (no state change, no value) and `&mut self` as `payable`, which is
/// the least restrictive mutable form and preserves the historical behavior.
///
/// # Arguments
///
/// * `attrs` - The attributes of the method
/// * `sig` - The signature of the method
///
/// # Returns
///
/// The resolved state mutability, or an error if the attribute is malformed or
/// contradicts the receiver
pub fn resolve_state_mutability(
    attrs: &[Attribute],
    sig: &Signature,
) -> syn::Result<StateMutability> {
    let receiver_mutability = mutability_from_receiver(sig);

    let Some(attr) = attrs
        .iter()
        .find(|attr| attr.path().is_ident(STATE_MUTABILITY_ATTR))
    else {
        return Ok(receiver_mutability);
    };

    let literal = attr.parse_args::<LitStr>().map_err(|_| {
        syn::Error::new(
            attr.span(),
            format!(
                "Expected #[{}(\"...\")] with one of: {}",
                STATE_MUTABILITY_ATTR,
                VALID_MUTABILITIES.join(", ")
            ),
        )
    })?;

    let declared = StateMutability::from_str(&literal.value()).ok_or_else(|| {
        syn::Error::new(
            literal.span(),
            format!(
                "Invalid state mutability '{}'. Valid values are: {}",
                literal.value(),
                VALID_MUTABILITIES.join(", ")
            ),
        )
    })?;

    // A `&mut self` receiver promises state access, so it cannot be reconciled
    // with a read-only mutability: the two would disagree on the host operation.
    if declared.is_static() && has_mutable_receiver(sig) {
        return Err(syn::Error::new(
            literal.span(),
            format!(
                "Method declared as '{}' must take '&self', not '&mut self'",
                literal.value()
            ),
        ));
    }

    Ok(declared)
}

/// Returns the mutability implied by a method receiver
fn mutability_from_receiver(sig: &Signature) -> StateMutability {
    if has_mutable_receiver(sig) {
        StateMutability::Payable
    } else {
        StateMutability::View
    }
}

/// Returns true if the method takes `&mut self`
fn has_mutable_receiver(sig: &Signature) -> bool {
    sig.inputs.iter().any(|arg| match arg {
        FnArg::Receiver(receiver) => receiver.mutability.is_some(),
        FnArg::Typed(_) => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::{parse_quote, TraitItemFn};

    fn resolve(method: &TraitItemFn) -> syn::Result<StateMutability> {
        resolve_state_mutability(&method.attrs, &method.sig)
    }

    #[test]
    fn test_receiver_drives_default_mutability() {
        let immutable: TraitItemFn = parse_quote! {
            fn balance_of(&self, owner: Address) -> U256;
        };
        assert_eq!(resolve(&immutable).unwrap(), StateMutability::View);

        let mutable: TraitItemFn = parse_quote! {
            fn transfer(&mut self, to: Address) -> bool;
        };
        assert_eq!(resolve(&mutable).unwrap(), StateMutability::Payable);
    }

    #[test]
    fn test_attribute_overrides_receiver() {
        let nonpayable: TraitItemFn = parse_quote! {
            #[state_mutability("nonpayable")]
            fn transfer(&mut self, to: Address) -> bool;
        };
        assert_eq!(resolve(&nonpayable).unwrap(), StateMutability::NonPayable);

        let pure_fn: TraitItemFn = parse_quote! {
            #[state_mutability("pure")]
            fn add(&self, a: U256, b: U256) -> U256;
        };
        assert_eq!(resolve(&pure_fn).unwrap(), StateMutability::Pure);
    }

    #[test]
    fn test_read_only_mutability_rejects_mutable_receiver() {
        let contradiction: TraitItemFn = parse_quote! {
            #[state_mutability("view")]
            fn balance_of(&mut self, owner: Address) -> U256;
        };
        let err = resolve(&contradiction).unwrap_err();
        assert!(
            err.to_string().contains("must take '&self'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_invalid_mutability_is_rejected() {
        let unknown: TraitItemFn = parse_quote! {
            #[state_mutability("constant")]
            fn balance_of(&self, owner: Address) -> U256;
        };
        let err = resolve(&unknown).unwrap_err();
        assert!(
            err.to_string().contains("Invalid state mutability"),
            "unexpected error: {err}"
        );

        let malformed: TraitItemFn = parse_quote! {
            #[state_mutability(view)]
            fn balance_of(&self, owner: Address) -> U256;
        };
        let err = resolve(&malformed).unwrap_err();
        assert!(
            err.to_string().contains("Expected #[state_mutability"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_call_policy_per_mutability() {
        assert!(StateMutability::Pure.is_static());
        assert!(StateMutability::View.is_static());
        assert!(!StateMutability::NonPayable.is_static());
        assert!(!StateMutability::Payable.is_static());

        assert!(!StateMutability::Pure.allows_value());
        assert!(!StateMutability::View.allows_value());
        assert!(!StateMutability::NonPayable.allows_value());
        assert!(StateMutability::Payable.allows_value());
    }
}
