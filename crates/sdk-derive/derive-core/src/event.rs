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
//!
//! Struct-typed fields are expanded into their components before the signature is hashed, the
//! same way the router expands struct parameters (see [`crate::abi::structs`]): a field of type
//! `Order { amount: U256, filled: bool }` contributes `(uint256,bool)`, so `topics[0]` matches
//! what a Solidity subscriber computes from the canonical signature.

use crate::abi::{
    error::ABIError, parameter::Parameter, structs::StructResolver, types::rust_to_sol,
};
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
///
/// Struct-typed fields are expanded from the sources of the crate being compiled, so the
/// signature baked into `SELECTOR` is the one callers derive from the published ABI.
pub fn process_event(input: DeriveInput) -> Result<TokenStream2> {
    process_event_with_structs(input, &StructResolver::crate_sources())
}

/// Parses and validates an event, resolving struct-typed fields through the given resolver.
pub fn process_event_with_structs(
    input: DeriveInput,
    resolver: &StructResolver,
) -> Result<TokenStream2> {
    let event = parse_event(input)?;
    validate_event(&event)?;
    generate_event_impl(&event, resolver)
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

    Ok(ParsedEvent {
        name,
        fields,
        anonymous,
    })
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
                if event.anonymous {
                    "anonymous"
                } else {
                    "regular"
                }
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

/// Canonical Solidity signature of the event: `EventName(type1,type2,...)`
///
/// A struct field renders as `tuple` until its components are known, and `keccak256` of that
/// string is a topic no canonical filter matches, so every struct is expanded through the resolver
/// first. An unresolvable struct is an error rather than a silently wrong `topics[0]`.
fn event_signature(event: &ParsedEvent, resolver: &StructResolver) -> Result<String> {
    let mut parameters = event
        .fields
        .iter()
        .map(|f| {
            Parameter::from_rust_type(f.name.to_string(), &f.ty)
                .map_err(|e| Error::new_spanned(&f.ty, format!("Cannot convert type: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    if parameters.iter().any(Parameter::has_unresolved_struct) {
        let resolution_error = |error: ABIError| {
            Error::new(
                Span::call_site(),
                format!("event `{}`: {error}", event.name),
            )
        };
        let structs = resolver.structs().map_err(resolution_error)?;
        for parameter in &mut parameters {
            // Events live in the crate root scope, the same as contract signatures
            parameter
                .resolve_structs(structs, "")
                .map_err(resolution_error)?;
        }
    }

    let sol_types = parameters
        .iter()
        .map(|parameter| {
            parameter
                .get_canonical_type()
                .map_err(|e| Error::new(Span::call_site(), format!("event `{}`: {e}", event.name)))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(format!("{}({})", event.name, sol_types.join(",")))
}

fn generate_event_impl(event: &ParsedEvent, resolver: &StructResolver) -> Result<TokenStream2> {
    let name = &event.name;

    let signature = event_signature(event, resolver)?;

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
            pub fn emit<SDK: fluentbase_sdk::SharedAPI>(&self, sdk: &mut SDK) -> Result<(), fluentbase_sdk::ExitCode> {
                let topics: [fluentbase_sdk::B256; #topic_count] = #topics_code;
                #data_code
                sdk.emit_log(&topics, &data).ok()
            }
        }
    })
}

/// Generates topic encoding for indexed fields.
///
/// Topics are 32-byte values used for bloom filter indexing, so Solidity splits indexed
/// parameters by *type category* rather than by whether the ABI encoding is dynamic:
/// - Value types (address, uint256, bool, bytesN, etc.): the ABI word is the topic.
/// - Reference types (string, bytes, arrays — fixed ones included — and structs): the topic is
///   keccak256 of a preimage that is not ordinary ABI encoding, hashed at runtime via
///   SDK::keccak256.
///
/// `fluentbase_sdk::codec::encode_indexed_topic` builds both cases; hashing stays here because it
/// goes through the host function rather than a software keccak256.
fn generate_topics(indexed: &[&EventField], anonymous: bool, selector: &[u8; 32]) -> TokenStream2 {
    let mut exprs = Vec::new();

    // topics[0] = event signature hash (unless anonymous)
    if !anonymous {
        exprs.push(quote! { fluentbase_sdk::B256::new([#(#selector),*]) });
    }

    for field in indexed {
        let name = &field.name;

        exprs.push(quote! {
            {
                let topic = fluentbase_sdk::codec::encode_indexed_topic(&self.#name)
                    .expect("encode indexed field");

                match topic {
                    fluentbase_sdk::codec::IndexedTopic::Word(word) => {
                        fluentbase_sdk::B256::new(word)
                    }
                    fluentbase_sdk::codec::IndexedTopic::Preimage(preimage) => {
                        fluentbase_sdk::B256::new(SDK::keccak256(&preimage).0)
                    }
                }
            }
        });
    }

    quote! { [#(#exprs),*] }
}

/// Generates data encoding for non-indexed fields.
///
/// Data section contains the non-indexed fields encoded with top-level argument semantics,
/// exactly like Solidity's `abi.encode(arg0, arg1, ...)`. This is *not* the same as encoding
/// the fields as a single tuple value: a dynamic tuple value carries an extra outer offset
/// word that standard log decoders do not expect, so `encode_function_args` is used to drop it.
///
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
            fluentbase_sdk::codec::SolidityABI::encode_function_args(&values, &mut buf)
                .expect("encode data fields");
            buf.freeze()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::structs::StructRegistry;
    use insta::assert_snapshot;
    use std::fs;
    use syn::parse_quote;
    use tempfile::TempDir;

    fn generate(input: DeriveInput) -> String {
        let tokens = process_event(input).unwrap();
        let file = syn::parse_file(&tokens.to_string()).unwrap();
        prettyplease::unparse(&file)
    }

    /// A resolver over a crate consisting of the given root source
    fn resolver_for(source: &str) -> (TempDir, StructResolver) {
        let temp_dir = TempDir::new().unwrap();
        let entry_file = temp_dir.path().join("lib.rs");
        fs::write(&entry_file, source).unwrap();
        let registry = StructRegistry::parse_crate(&entry_file).unwrap();
        (temp_dir, StructResolver::registry(registry))
    }

    /// Generates the event with struct fields resolved against `source`
    fn generate_with(input: DeriveInput, source: &str) -> String {
        let (_temp, resolver) = resolver_for(source);
        let tokens = process_event_with_structs(input, &resolver).unwrap();
        let file = syn::parse_file(&tokens.to_string()).unwrap();
        prettyplease::unparse(&file)
    }

    /// The `SELECTOR` bytes of a generated event, as a hex string
    fn selector_hex(generated: &str) -> String {
        let start =
            generated.find("SELECTOR: [u8; 32] = [").unwrap() + "SELECTOR: [u8; 32] = [".len();
        let end = start + generated[start..].find(']').unwrap();
        generated[start..end]
            .split(',')
            .map(str::trim)
            .filter(|byte| !byte.is_empty())
            .map(|byte| {
                let byte = byte.strip_suffix("u8").unwrap_or(byte);
                format!("{:02x}", byte.parse::<u8>().unwrap())
            })
            .collect()
    }

    const ORDER: &str = "#[derive(Codec)] pub struct Order { pub amount: U256, pub filled: bool }";

    /// A struct field hashes as its components, not as the literal `tuple`
    #[test]
    fn test_struct_field_signs_as_resolved_components() {
        let input: DeriveInput = parse_quote! {
            struct Filled {
                #[indexed]
                who: Address,
                order: Order,
            }
        };
        let generated = generate_with(input, ORDER);
        assert!(
            generated.contains("\"Filled(address,(uint256,bool))\""),
            "unexpected signature: {generated}"
        );
        // keccak256("Filled(address,(uint256,bool))")
        assert_eq!(
            selector_hex(&generated),
            hex::encode(keccak256(b"Filled(address,(uint256,bool))"))
        );
    }

    /// Struct arrays, fixed or dynamic, expand the element type the same way
    #[test]
    fn test_struct_array_fields_sign_as_resolved_components() {
        let input: DeriveInput = parse_quote! {
            struct Batched {
                orders: Vec<Order>,
                pair: [Order; 2],
            }
        };
        let generated = generate_with(input, ORDER);
        assert!(
            generated.contains("\"Batched((uint256,bool)[],(uint256,bool)[2])\""),
            "unexpected signature: {generated}"
        );
    }

    /// Nested structs are expanded recursively, from the module the outer struct lives in
    #[test]
    fn test_nested_struct_field_signs_as_resolved_components() {
        let input: DeriveInput = parse_quote! {
            struct Filled {
                order: types::Order,
            }
        };
        let generated = generate_with(
            input,
            r#"
mod types {
    #[derive(Codec)]
    pub struct Price { pub base: U256, pub quote: U256 }

    #[derive(Codec)]
    pub struct Order { pub price: Price, pub filled: bool }
}
"#,
        );
        assert!(
            generated.contains("\"Filled(((uint256,uint256),bool))\""),
            "unexpected signature: {generated}"
        );
    }

    /// A struct the crate does not define has no canonical signature, so the event fails to
    /// build instead of hashing a `tuple` placeholder into `topics[0]`
    #[test]
    fn test_unresolved_struct_field_is_rejected() {
        let input: DeriveInput = parse_quote! {
            struct Filled {
                order: Order,
            }
        };
        let (_temp, resolver) = resolver_for("#[derive(Codec)] pub struct Other { pub v: U256 }");
        let error = process_event_with_structs(input, &resolver)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no `#[derive(Codec)]` definition"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("event `Filled`"),
            "unexpected error: {error}"
        );
    }

    /// Fields without structs never consult the crate sources
    #[test]
    fn test_primitive_fields_do_not_need_a_resolver() {
        let input: DeriveInput = parse_quote! {
            struct Transfer {
                #[indexed]
                from: Address,
                value: U256,
                pair: (Address, U256),
            }
        };
        let registry = StructRegistry::default();
        let generated = process_event_with_structs(input, &StructResolver::registry(registry))
            .unwrap()
            .to_string();
        assert!(generated.contains("\"Transfer(address,uint256,(address,uint256))\""));
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
    fn test_byte_array_fields_sign_as_uint8_arrays() {
        // `[u8; N]` is laid out one word per element by the codec and hashed as an array when
        // indexed, so the signature has to say `uint8[N]`; `bytesN` belongs to `FixedBytes<N>`.
        let input: DeriveInput = parse_quote! {
            struct Tagged {
                #[indexed]
                tag: [u8; 4],
                digest: B256,
            }
        };
        assert!(generate(input).contains("\"Tagged(uint8[4],bytes32)\""));
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
    fn test_mixed_static_dynamic_data() {
        let input: DeriveInput = parse_quote! {
            struct Mixed {
                #[indexed]
                who: Address,
                amount: U256,
                note: String,
                extra: Vec<Address>,
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
