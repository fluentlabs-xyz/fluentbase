use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericParam};

pub fn process_storage_layout(input: DeriveInput) -> Result<TokenStream2, syn::Error> {
    let has_sdk =
        input.generics.params.iter().any(
            |param| matches!(param, GenericParam::Type(type_param) if type_param.ident == "SDK"),
        );

    if has_sdk {
        process_contract_storage(input)
    } else {
        process_composite_storage(input)
    }
}

fn process_contract_storage(input: DeriveInput) -> Result<TokenStream2, syn::Error> {
    let name = &input.ident;
    let fields = extract_storage_fields(&input)?;

    let constructor = generate_constructor_body(&fields, true);
    let accessors = generate_accessor_methods(&fields, true);
    let slots_calculation = generate_const_slots_calculation(&fields);

    Ok(quote! {
        impl<SDK> #name<SDK> {
            pub const REQUIRED_SLOTS: usize = Self::calculate_required_slots();

            pub fn new(sdk: SDK, slot: fluentbase_sdk::U256, offset: u8) -> Self {
                #constructor
            }

            #slots_calculation
            #accessors
        }
    })
}

fn process_composite_storage(input: DeriveInput) -> Result<TokenStream2, syn::Error> {
    let name = &input.ident;
    let fields = extract_storage_fields(&input)?;

    let slots_calculation = generate_const_slots_calculation(&fields);
    let constructor = generate_constructor_body(&fields, false);
    let accessors = generate_accessor_methods(&fields, false);

    Ok(quote! {
        impl #name {
            pub const REQUIRED_SLOTS: usize = Self::calculate_required_slots();

            pub fn new(slot: fluentbase_sdk::U256, offset: u8) -> Self {
                #constructor
            }

            #slots_calculation
            #accessors
        }

        impl fluentbase_sdk::storage::composite::CompositeStorage for #name {
            const REQUIRED_SLOTS: usize = Self::REQUIRED_SLOTS;

            fn from_slot(base_slot: fluentbase_sdk::U256) -> Self {
                Self::new(base_slot, 0)
            }
        }
    })
}

fn generate_constructor_body(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    has_sdk: bool,
) -> TokenStream2 {
    let mut layout_calculations = Vec::new();
    let mut field_assignments = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().expect("Named fields required");

        if has_sdk && field_name == "sdk" {
            continue;
        }

        let field_type = &field.ty;
        let layout_var = quote::format_ident!("{}_layout", field_name);

        layout_calculations.push(generate_layout_calculation(&layout_var, field_type));
        field_assignments.push(generate_field_init(field_name, field_type, &layout_var));
    }

    let sdk_assignment = if has_sdk {
        quote! { sdk, }
    } else {
        quote! {}
    };

    quote! {
        let mut current_slot = slot;
        let mut current_offset: u8 = offset;

        #(#layout_calculations)*

        Self {
            #sdk_assignment
            #(#field_assignments),*
        }
    }
}

fn generate_layout_calculation(layout_var: &syn::Ident, field_type: &syn::Type) -> TokenStream2 {
    quote! {
        let #layout_var = {
            let encoded_size = <#field_type as fluentbase_sdk::storage::StorageLayout>::ENCODED_SIZE as u8;
            let required_slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::REQUIRED_SLOTS;

            let layout = if required_slots == 0 {
                // StoragePrimitive type - try to pack
                if current_offset + encoded_size <= 32 {
                    // Fits in current slot
                    // Calculate offset from the RIGHT edge (Solidity packs right to left)
                    let actual_offset = 32 - current_offset - encoded_size;
                    let result = (current_slot, actual_offset);
                    current_offset += encoded_size;
                    result
                } else {
                    // Doesn't fit, move to next slot
                    current_slot = current_slot + fluentbase_sdk::U256::from(1);
                    // First element in new slot goes to rightmost position
                    let actual_offset = 32 - encoded_size;
                    current_offset = encoded_size;
                    (current_slot, actual_offset)
                }
            } else {
                // Composite type - needs its own slot(s)
                if current_offset > 0 {
                    // If we were packing, move to next slot
                    current_slot = current_slot + fluentbase_sdk::U256::from(1);
                    current_offset = 0;
                }
                let result = (current_slot, 0);
                current_slot = current_slot + fluentbase_sdk::U256::from(required_slots);
                current_offset = 0;
                result
            };

            layout
        };
    }
}

fn generate_field_init(
    field_name: &syn::Ident,
    field_type: &syn::Type,
    layout_var: &syn::Ident,
) -> TokenStream2 {
    quote! {
        #field_name: <<#field_type as fluentbase_sdk::storage::StorageLayout>::Descriptor as fluentbase_sdk::storage::StorageDescriptor>::new(
            #layout_var.0,
            #layout_var.1
        )
    }
}

fn generate_const_slots_calculation(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> TokenStream2 {
    let mut field_calculations = Vec::new();

    for field in fields {
        let field_type = &field.ty;

        field_calculations.push(quote! {
            {
                let encoded_size = <#field_type as fluentbase_sdk::storage::StorageLayout>::ENCODED_SIZE;
                let required_slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::REQUIRED_SLOTS;

                if required_slots == 0 {
                    // StoragePrimitive type
                    if current_offset + encoded_size <= 32 {
                        current_offset += encoded_size;
                    } else {
                        current_slot += 1;
                        current_offset = encoded_size;
                    }
                } else {
                    // Composite type
                    if current_offset > 0 {
                        current_slot += 1;
                        current_offset = 0;
                    }
                    current_slot += required_slots;
                    current_offset = 0;
                }
            }
        });
    }

    quote! {
        const fn calculate_required_slots() -> usize {
            let mut current_slot: usize = 0;
            let mut current_offset: usize = 0;

            #(#field_calculations)*

            if current_offset > 0 {
                current_slot + 1
            } else {
                current_slot
            }
        }
    }
}

// Keep the existing helper functions unchanged
fn extract_named_fields(
    input: &DeriveInput,
) -> Result<&syn::punctuated::Punctuated<syn::Field, syn::token::Comma>, syn::Error> {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(&fields.named),
            _ => Err(syn::Error::new_spanned(
                input,
                "StorageLayout only supports structs with named fields",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            input,
            "StorageLayout can only be derived for structs",
        )),
    }
}

fn extract_storage_fields(
    input: &DeriveInput,
) -> Result<syn::punctuated::Punctuated<syn::Field, syn::token::Comma>, syn::Error> {
    let fields = extract_named_fields(input)?;

    let mut storage_fields = syn::punctuated::Punctuated::new();
    for field in fields {
        if let Some(field_name) = &field.ident {
            if field_name != "sdk" {
                storage_fields.push(field.clone());
            }
        }
    }
    Ok(storage_fields)
}

fn generate_accessor_methods(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    has_sdk: bool,
) -> TokenStream2 {
    let mut methods = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().expect("Named fields required");

        if has_sdk && field_name == "sdk" {
            continue;
        }

        let field_type = &field.ty;
        let doc_string = format!("Returns an accessor for the {} field", field_name);

        methods.push(quote! {
            #[doc = #doc_string]
            #[inline]
            pub fn #field_name(&self) -> <#field_type as fluentbase_sdk::storage::StorageLayout>::Accessor {
                <#field_type as fluentbase_sdk::storage::StorageLayout>::access(self.#field_name)
            }
        });
    }

    quote! {
        #(#methods)*
    }
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
