//! Event derive macro for Solidity-compatible event emission.
//!
//! Generates `emit()` method that produces EVM logs matching Solidity event ABI.
//!
//! # Example
//! ```ignore
//! #[derive(Event)]
//! struct Transfer {
//!     #[indexed]
//!     from: Address,
//!     #[indexed]
//!     to: Address,
//!     value: U256,
//! }
//!
//! // Emits log with:
//! // topics[0] = keccak256("Transfer(address,address,uint256)")
//! // topics[1] = from
//! // topics[2] = to
//! // data = abi.encode(value)
//! Transfer { from, to, value }.emit(&mut sdk);
//! ```

use crate::abi::types::rust_to_sol;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, Result, Type};

/// EVM allows maximum 4 topics per log.
/// Regular events use topic[0] for signature, leaving 3 for indexed fields.
const MAX_INDEXED_FIELDS: usize = 3;

/// Anonymous events have no signature topic, all 4 available for indexed fields.
const MAX_INDEXED_FIELDS_ANONYMOUS: usize = 4;

struct EventField {
    name: Ident,
    ty: Type,
    /// Indexed fields go to topics (filterable via bloom filter).
    /// Non-indexed fields go to data (cheaper, but requires full scan to filter).
    indexed: bool,
}

struct ParsedEvent {
    name: Ident,
    fields: Vec<EventField>,
    /// Anonymous events omit signature from topics, saving gas and allowing 4 indexed fields.
    anonymous: bool,
}

/// Main entry point for the Event derive macro.
pub fn process_event(input: DeriveInput) -> Result<TokenStream2> {
    let event = parse_event(input)?;
    validate_event(&event)?;
    generate_event_impl(&event)
}

fn parse_event(input: DeriveInput) -> Result<ParsedEvent> {
    let name = input.ident;
    let anonymous = has_attribute(&input.attrs, "anonymous");

    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(named) => named
                .named
                .into_iter()
                .map(|f| {
                    // Validate type is convertible to Solidity (for signature generation)
                    rust_to_sol(&f.ty).map_err(|e| {
                        Error::new_spanned(&f.ty, format!("Cannot convert type: {}", e))
                    })?;

                    Ok(EventField {
                        name: f.ident.expect("Named field"),
                        ty: f.ty,
                        indexed: has_attribute(&f.attrs, "indexed"),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(Error::new(Span::call_site(), "Only named fields supported")),
        },
        _ => return Err(Error::new(Span::call_site(), "Only structs supported")),
    };

    Ok(ParsedEvent { name, fields, anonymous })
}

fn has_attribute(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn validate_event(event: &ParsedEvent) -> Result<()> {
    let indexed_count = event.fields.iter().filter(|f| f.indexed).count();
    let max = if event.anonymous {
        MAX_INDEXED_FIELDS_ANONYMOUS
    } else {
        MAX_INDEXED_FIELDS
    };

    if indexed_count > max {
        return Err(Error::new(
            Span::call_site(),
            format!(
                "Too many indexed fields: {} (max {} for {} events)",
                indexed_count,
                max,
                if event.anonymous { "anonymous" } else { "regular" }
            ),
        ));
    }
    Ok(())
}

/// Compile-time keccak256 for event signature hashing.
fn keccak256(input: &[u8]) -> [u8; 32] {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hasher = Keccak256::new();
    hasher.update(input);
    let mut output = [0u8; 32];
    output.copy_from_slice(&hasher.finalize());
    output
}

fn generate_event_impl(event: &ParsedEvent) -> Result<TokenStream2> {
    let name = &event.name;

    // Build Solidity signature: "EventName(type1,type2,...)"
    let sol_types: Vec<String> = event
        .fields
        .iter()
        .map(|f| {
            rust_to_sol(&f.ty)
                .map(|t| t.abi_type())
                .unwrap_or_else(|_| "unknown".to_string())
        })
        .collect();
    let signature = format!("{}({})", name, sol_types.join(","));

    // Computed at compile-time by proc-macro
    let selector = keccak256(signature.as_bytes());

    let indexed: Vec<_> = event.fields.iter().filter(|f| f.indexed).collect();
    let data_fields: Vec<_> = event.fields.iter().filter(|f| !f.indexed).collect();

    let topic_count = indexed.len() + if event.anonymous { 0 } else { 1 };
    let topics_code = generate_topics(&indexed, event.anonymous, &selector);
    let data_code = generate_data(&data_fields);

    Ok(quote! {
        impl #name {
            /// Solidity event signature.
            pub const SIGNATURE: &'static str = #signature;

            /// Keccak256 hash of signature, computed at compile-time.
            pub const SELECTOR: [u8; 32] = [#(#selector),*];

            /// Emits this event as an EVM log.
            pub fn emit<SDK: fluentbase_sdk::SharedAPI>(&self, sdk: &mut SDK) {
                use fluentbase_sdk::codec::SolidityABI;

                let topics: [fluentbase_sdk::B256; #topic_count] = #topics_code;
                #data_code
                sdk.emit_log(&topics, &data);
            }
        }
    })
}

/// Generates topic encoding for indexed fields.
///
/// Topics are 32-byte values used for bloom filter indexing:
/// - Static types (address, uint256, bool, etc.): ABI-encoded directly (always 32 bytes)
/// - Dynamic types (string, bytes, arrays, dynamic structs): keccak256 of ABI-encoded value
///   (computed at runtime via SDK::keccak256)
///
/// This distinction exists because bloom filters require fixed-size data.
/// Dynamic types are hashed, losing original value but enabling equality filtering.
fn generate_topics(indexed: &[&EventField], anonymous: bool, selector: &[u8; 32]) -> TokenStream2 {
    let mut exprs = Vec::new();

    // topics[0] = event signature hash (unless anonymous)
    if !anonymous {
        exprs.push(quote! { fluentbase_sdk::B256::new([#(#selector),*]) });
    }

    for field in indexed {
        let name = &field.name;
        let ty = &field.ty;

        exprs.push(quote! {
            {
                let mut buf = fluentbase_sdk::codec::bytes::BytesMut::new();
                let value = self.#name.clone();
                SolidityABI::encode(&value, &mut buf, 0).expect("encode indexed field");

                if SolidityABI::<#ty>::is_dynamic() {
                    // Dynamic type: hash at runtime via SDK
                    fluentbase_sdk::B256::new(SDK::keccak256(&buf).0)
                } else {
                    // Static type: use ABI-encoded value directly (32 bytes)
                    let mut bytes = [0u8; 32];
                    bytes.copy_from_slice(&buf[..32]);
                    fluentbase_sdk::B256::new(bytes)
                }
            }
        });
    }

    quote! { [#(#exprs),*] }
}

/// Generates data encoding for non-indexed fields.
///
/// Data section contains ABI-encoded tuple of all non-indexed fields.
/// Unlike topics, data preserves full values but cannot be filtered via bloom filter.
fn generate_data(fields: &[&EventField]) -> TokenStream2 {
    if fields.is_empty() {
        return quote! { let data: &[u8] = &[]; };
    }

    let names: Vec<_> = fields.iter().map(|f| &f.name).collect();

    quote! {
        let data = {
            let mut buf = fluentbase_sdk::codec::bytes::BytesMut::new();
            let values = (#(self.#names.clone(),)*);
            SolidityABI::encode(&values, &mut buf, 0).expect("encode data fields");
            buf.freeze()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use syn::parse_quote;

    fn generate(input: DeriveInput) -> String {
        let tokens = process_event(input).unwrap();
        let file = syn::parse_file(&tokens.to_string()).unwrap();
        prettyplease::unparse(&file)
    }

    #[test]
    fn test_basic_transfer() {
        let input: DeriveInput = parse_quote! {
            struct Transfer {
                #[indexed]
                from: Address,
                #[indexed]
                to: Address,
                value: U256,
            }
        };
        assert_snapshot!(generate(input));
    }

    #[test]
    fn test_all_indexed() {
        let input: DeriveInput = parse_quote! {
            struct Approval {
                #[indexed]
                owner: Address,
                #[indexed]
                spender: Address,
                #[indexed]
                value: U256,
            }
        };
        assert_snapshot!(generate(input));
    }

    #[test]
    fn test_no_indexed() {
        let input: DeriveInput = parse_quote! {
            struct DataStored {
                key: U256,
                value: U256,
            }
        };
        assert_snapshot!(generate(input));
    }

    #[test]
    fn test_anonymous() {
        let input: DeriveInput = parse_quote! {
            #[anonymous]
            struct Anonymous {
                #[indexed]
                a: Address,
                #[indexed]
                b: Address,
                #[indexed]
                c: Address,
                #[indexed]
                d: Address,
            }
        };
        assert_snapshot!(generate(input));
    }

    #[test]
    fn test_dynamic_indexed() {
        let input: DeriveInput = parse_quote! {
            struct Message {
                #[indexed]
                sender: Address,
                #[indexed]
                text: String,
            }
        };
        assert_snapshot!(generate(input));
    }

    #[test]
    fn test_too_many_indexed_regular() {
        let input: DeriveInput = parse_quote! {
            struct TooMany {
                #[indexed]
                a: Address,
                #[indexed]
                b: Address,
                #[indexed]
                c: Address,
                #[indexed]
                d: Address,
            }
        };
        let err = process_event(input).unwrap_err();
        assert!(err.to_string().contains("Too many indexed"));
    }

    #[test]
    fn test_too_many_indexed_anonymous() {
        let input: DeriveInput = parse_quote! {
            #[anonymous]
            struct TooManyAnon {
                #[indexed]
                a: Address,
                #[indexed]
                b: Address,
                #[indexed]
                c: Address,
                #[indexed]
                d: Address,
                #[indexed]
                e: Address,
            }
        };
        let err = process_event(input).unwrap_err();
        assert!(err.to_string().contains("Too many indexed"));
    }
}