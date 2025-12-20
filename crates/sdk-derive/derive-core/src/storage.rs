use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::{Data, DeriveInput, Fields, GenericParam};
use syn::spanned::Spanned;

/// Processes a struct with `#[derive(Storage)]` or `#[derive(Contract)]` attribute.
///
/// Generates Solidity-compatible storage layout with:
/// - Automatic slot packing for small types
/// - Sequential slot allocation
/// - Support for explicit `#[slot(U256)]` positioning
/// - Accessor methods for each field
pub fn process_storage_layout(input: DeriveInput) -> Result<TokenStream2, syn::Error> {
    let name = &input.ident;
    let has_sdk = input.generics.params.iter().any(
        |param| matches!(param, GenericParam::Type(type_param) if type_param.ident == "SDK"),
    );

    let fields = extract_storage_fields(&input)?;

    let slots_calculation = generate_const_slots_calculation(&fields)?;
    let bytes_calculation = generate_const_bytes_calculation(&fields)?;
    let constructor = generate_constructor_body(&fields, has_sdk)?;
    let accessors = generate_accessor_methods(&fields);

    if has_sdk {
        // Contract with SDK - generate impl block only
        Ok(quote! {
            impl<SDK> #name<SDK> {
                /// Number of storage slots used by auto-layout fields.
                /// Fields with explicit #[slot()] are not counted.
                pub const SLOTS: usize = Self::calculate_slots();

                /// Creates a new instance starting at slot 0.
                pub fn new(sdk: SDK) -> Self {
                    let slot = fluentbase_sdk::U256::from(0);
                    let offset = 0u8;
                    #constructor
                }

                /// Creates a new instance at a specific slot and offset.
                pub fn new_at(sdk: SDK, slot: fluentbase_sdk::U256, offset: u8) -> Self {
                    #constructor
                }

                #slots_calculation
                #accessors
            }
        })
    } else {
        // Regular storage struct - generate full trait implementations
        let first_field = fields.first().ok_or_else(|| {
            syn::Error::new_spanned(&input, "Storage struct requires at least one field")
        })?;
        let first_field_name = first_field.ident.as_ref().unwrap();

        Ok(quote! {
            impl #name {
                /// Number of storage slots used by auto-layout fields.
                pub const SLOTS: usize = Self::calculate_slots();

                /// Creates a new instance at a specific slot and offset.
                pub fn new(slot: fluentbase_sdk::U256, offset: u8) -> Self {
                    #constructor
                }

                #slots_calculation
                #bytes_calculation
                #accessors
            }

            impl Copy for #name {}

            impl Clone for #name {
                fn clone(&self) -> Self {
                    *self
                }
            }

            impl fluentbase_sdk::storage::StorageDescriptor for #name {
                fn new(slot: fluentbase_sdk::U256, offset: u8) -> Self {
                    Self::new(slot, offset)
                }

                fn slot(&self) -> fluentbase_sdk::U256 {
                    self.#first_field_name.slot()
                }

                fn offset(&self) -> u8 {
                    0
                }
            }

            impl fluentbase_sdk::storage::StorageLayout for #name {
                type Descriptor = Self;
                type Accessor = Self;

                const BYTES: usize = Self::calculate_bytes();
                const SLOTS: usize = Self::calculate_slots();

                fn access(descriptor: Self::Descriptor) -> Self::Accessor {
                    descriptor
                }
            }
        })
    }
}

/// Extracts `#[slot(expr)]` attribute value if present.
///
/// Returns:
/// - `Ok(Some(tokens))` if valid #[slot(expr)] found
/// - `Ok(None)` if no slot attribute
/// - `Err` for malformed attributes
fn get_slot_attr(field: &syn::Field) -> Result<Option<TokenStream2>, syn::Error> {
    for attr in &field.attrs {
        if attr.path().is_ident("slot") {
            match &attr.meta {
                syn::Meta::List(list) => {
                    if list.tokens.is_empty() {
                        return Err(syn::Error::new_spanned(
                            attr,
                            "#[slot(...)] requires a U256 expression, e.g. #[slot(MY_SLOT)]",
                        ));
                    }
                    return Ok(Some(list.tokens.clone()));
                }
                syn::Meta::Path(_) => {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "#[slot] requires an argument, e.g. #[slot(MY_SLOT)]",
                    ));
                }
                syn::Meta::NameValue(_) => {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "use #[slot(expr)] not #[slot = expr]",
                    ));
                }
            }
        }
    }
    Ok(None)
}

/// Checks if a field has an explicit slot attribute.
fn has_explicit_slot(field: &syn::Field) -> bool {
    matches!(get_slot_attr(field), Ok(Some(_)))
}

/// Generates the constructor body that initializes all storage fields.
fn generate_constructor_body(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    has_sdk: bool,
) -> Result<TokenStream2, syn::Error> {
    let mut layout_calculations = Vec::new();
    let mut field_assignments = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().expect("Named fields required");
        let field_type = &field.ty;
        let layout_var = quote::format_ident!("{}_layout", field_name);

        layout_calculations.push(generate_layout_calculation(field, &layout_var, field_type)?);
        field_assignments.push(generate_field_init(field_name, field_type, &layout_var));
    }

    let sdk_assignment = if has_sdk {
        quote! { sdk, }
    } else {
        quote! {}
    };

    Ok(quote! {
        let mut current_slot = slot;
        let mut current_offset: u8 = offset;

        #(#layout_calculations)*

        Self {
            #sdk_assignment
            #(#field_assignments),*
        }
    })
}


/// Generates layout calculation for a single field.
///
/// For explicit `#[slot(expr)]`: uses the provided slot, does not affect auto-layout state.
/// For auto-layout: calculates slot/offset based on current position and field size.
fn generate_layout_calculation(
    field: &syn::Field,
    layout_var: &syn::Ident,
    field_type: &syn::Type,
) -> Result<TokenStream2, syn::Error> {
    if let Some(slot_expr) = get_slot_attr(field)? {
        // Use span from the slot expression for better error messages
        let span = slot_expr.span();

        // Explicit slot positioning
        // quote_spanned ensures type mismatch errors point to the attribute
        return Ok(quote_spanned! {span=>
            let #layout_var = {
                let bytes = <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES as u8;
                let slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::SLOTS;

                // Compile-time type check: must be U256
                let explicit_slot: fluentbase_sdk::U256 = #slot_expr;

                // Packable types: right-align within slot (Solidity convention)
                // Full-slot types: start at offset 0
                let offset = if slots == 0 { 32 - bytes } else { 0 };

                (explicit_slot, offset)
            };
        });
    }

    // Auto-layout with packing
    Ok(quote! {
        let #layout_var = {
            let bytes = <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES as u8;
            let slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::SLOTS;

            if slots == 0 {
                // Packable type (fits within a slot)
                if current_offset + bytes <= 32 {
                    // Fits in current slot - right-align
                    let actual_offset = 32 - current_offset - bytes;
                    let result = (current_slot, actual_offset);
                    current_offset += bytes;
                    result
                } else {
                    // Move to next slot
                    current_slot = current_slot + fluentbase_sdk::U256::from(1);
                    let actual_offset = 32 - bytes;
                    current_offset = bytes;
                    (current_slot, actual_offset)
                }
            } else {
                // Full-slot type (maps, arrays, large structs)
                if current_offset > 0 {
                    current_slot = current_slot + fluentbase_sdk::U256::from(1);
                    current_offset = 0;
                }
                let result = (current_slot, 0);
                current_slot = current_slot + fluentbase_sdk::U256::from(slots);
                result
            }
        };
    })
}

/// Generates field initialization from calculated layout.
fn generate_field_init(
    field_name: &syn::Ident,
    field_type: &syn::Type,
    layout_var: &syn::Ident,
) -> TokenStream2 {
    quote! {
        #field_name: <<#field_type as fluentbase_sdk::storage::StorageLayout>::Descriptor
            as fluentbase_sdk::storage::StorageDescriptor>::new(#layout_var.0, #layout_var.1)
    }
}

/// Generates accessor methods for all storage fields.
fn generate_accessor_methods(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> TokenStream2 {
    let methods = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().expect("Named fields required");
        let field_type = &field.ty;
        let method_name = quote::format_ident!("{}_accessor", field_name);
        let doc = format!("Returns an accessor for the `{}` storage field.", field_name);

        quote! {
            #[doc = #doc]
            #[inline]
            pub fn #method_name(&self) -> <#field_type as fluentbase_sdk::storage::StorageLayout>::Accessor {
                <#field_type as fluentbase_sdk::storage::StorageLayout>::access(self.#field_name)
            }
        }
    });

    quote! { #(#methods)* }
}

/// Generates const fn to calculate total slots used by auto-layout fields.
/// Fields with explicit `#[slot()]` are excluded.
fn generate_const_slots_calculation(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<TokenStream2, syn::Error> {
    let field_calculations: Vec<_> = fields
        .iter()
        .filter(|field| !has_explicit_slot(field))
        .map(|field| {
            let field_type = &field.ty;
            quote! {
                {
                    let bytes = <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES;
                    let slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::SLOTS;

                    if slots == 0 {
                        if current_offset + bytes <= 32 {
                            current_offset += bytes;
                        } else {
                            current_slot += 1;
                            current_offset = bytes;
                        }
                    } else {
                        if current_offset > 0 {
                            current_slot += 1;
                            current_offset = 0;
                        }
                        current_slot += slots;
                        current_offset = 0;
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        const fn calculate_slots() -> usize {
            let mut current_slot: usize = 0;
            let mut current_offset: usize = 0;

            #(#field_calculations)*

            if current_offset > 0 {
                current_slot + 1
            } else {
                current_slot
            }
        }
    })
}

/// Generates const fn to calculate total bytes used by auto-layout fields.
fn generate_const_bytes_calculation(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<TokenStream2, syn::Error> {
    let field_calculations: Vec<_> = fields
        .iter()
        .filter(|field| !has_explicit_slot(field))
        .map(|field| {
            let field_type = &field.ty;
            quote! {
                total_bytes += <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES;
            }
        })
        .collect();

    Ok(quote! {
        const fn calculate_bytes() -> usize {
            let mut total_bytes: usize = 0;
            #(#field_calculations)*
            total_bytes
        }
    })
}

/// Extracts named fields from a struct definition.
fn extract_named_fields(
    input: &DeriveInput,
) -> Result<&syn::punctuated::Punctuated<syn::Field, syn::token::Comma>, syn::Error> {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(&fields.named),
            _ => Err(syn::Error::new_spanned(
                input,
                "Storage macro only supports structs with named fields",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            input,
            "Storage macro can only be derived for structs",
        )),
    }
}

/// Extracts storage fields, excluding the `sdk` field if present.
fn extract_storage_fields(
    input: &DeriveInput,
) -> Result<syn::punctuated::Punctuated<syn::Field, syn::token::Comma>, syn::Error> {
    let fields = extract_named_fields(input)?;

    let storage_fields: syn::punctuated::Punctuated<_, _> = fields
        .iter()
        .filter(|field| {
            field
                .ident
                .as_ref()
                .map(|name| name != "sdk")
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    Ok(storage_fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use syn::{parse_file, parse_quote};

    #[test]
    fn test_composite_type() {
        let input: DeriveInput = parse_quote! {
            struct Config {
                owner: StoragePrimitive<Address>,
                version: StoragePrimitive<u32>,
                max_supply: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("composite_config", formatted);
    }

    #[test]
    fn test_contract_with_sdk() {
        let input: DeriveInput = parse_quote! {
            struct Storage<SDK> {
                sdk: SDK,
                owner: StoragePrimitive<Address>,
                counter: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("contract_storage", formatted);
    }

    #[test]
    fn test_nested_composite() {
        let input: DeriveInput = parse_quote! {
            struct NestedStructTest<SDK> {
                sdk: SDK,
                counter: StoragePrimitive<U256>,
                config: Composite<Config>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("nested_composite", formatted);
    }

    #[test]
    fn test_packed_composite() {
        let input: DeriveInput = parse_quote! {
            struct PackedConfig {
                is_active: StoragePrimitive<bool>,
                is_paused: StoragePrimitive<bool>,
                version: StoragePrimitive<u32>,
                flags: StoragePrimitive<u64>,
                owner: StoragePrimitive<Address>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("packed_composite", formatted);
    }

    #[test]
    fn test_explicit_slot_with_const() {
        let input: DeriveInput = parse_quote! {
            struct Proxy<SDK> {
                sdk: SDK,
                #[slot(IMPLEMENTATION_SLOT)]
                implementation: StoragePrimitive<Address>,
                #[slot(ADMIN_SLOT)]
                admin: StoragePrimitive<Address>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("explicit_slot_with_const", formatted);
    }

    #[test]
    fn test_mixed_auto_and_explicit_slots() {
        let input: DeriveInput = parse_quote! {
            struct MixedStorage<SDK> {
                sdk: SDK,
                auto1: StoragePrimitive<Address>,
                auto2: StoragePrimitive<U256>,
                #[slot(SPECIAL_SLOT)]
                explicit: StoragePrimitive<Address>,
                auto3: StoragePrimitive<bool>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("mixed_auto_and_explicit_slots", formatted);
    }

    #[test]
    fn test_all_explicit_slots() {
        let input: DeriveInput = parse_quote! {
            struct AllExplicit<SDK> {
                sdk: SDK,
                #[slot(SLOT_A)]
                field_a: StoragePrimitive<Address>,
                #[slot(SLOT_B)]
                field_b: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input).unwrap();
        let file = parse_file(&result.to_string()).unwrap();
        let formatted = prettyplease::unparse(&file);

        assert_snapshot!("all_explicit_slots", formatted);
    }

    #[test]
    fn test_slot_attr_empty_error() {
        let input: DeriveInput = parse_quote! {
            struct Bad<SDK> {
                sdk: SDK,
                #[slot()]
                value: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("requires"));
    }

    #[test]
    fn test_slot_attr_no_parens_error() {
        let input: DeriveInput = parse_quote! {
            struct Bad<SDK> {
                sdk: SDK,
                #[slot]
                value: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("requires an argument"));
    }

    #[test]
    fn test_slot_attr_name_value_error() {
        let input: DeriveInput = parse_quote! {
            struct Bad<SDK> {
                sdk: SDK,
                #[slot = "something"]
                value: StoragePrimitive<U256>,
            }
        };

        let result = process_storage_layout(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("#[slot(expr)]"));
    }

    #[test]
    fn test_unsupported_struct_type() {
        let input: DeriveInput = parse_quote! {
            struct Storage(SDK);
        };

        let result = process_storage_layout(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("named fields"));
    }

    #[test]
    fn test_enum_type() {
        let input: DeriveInput = parse_quote! {
            enum Storage {
                Variant1,
                Variant2,
            }
        };

        let result = process_storage_layout(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("structs"));
    }
}