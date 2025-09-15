use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericParam};

pub fn process_storage_layout(input: DeriveInput) -> Result<TokenStream2, syn::Error> {
    let name = &input.ident;
    let has_sdk =
        input.generics.params.iter().any(
            |param| matches!(param, GenericParam::Type(type_param) if type_param.ident == "SDK"),
        );

    // Extract storage fields (excluding sdk if present)
    let fields = extract_storage_fields(&input)?;

    // Generate the same storage logic for both cases
    let slots_calculation = generate_const_slots_calculation(&fields);
    let bytes_calculation = generate_const_bytes_calculation(&fields);
    let constructor = generate_constructor_body(&fields, has_sdk);
    let accessors = generate_accessor_methods(&fields);

    if has_sdk {
        // Contract with SDK - only generate impl block
        Ok(quote! {
            impl<SDK> #name<SDK> {
                pub const SLOTS: usize = Self::calculate_slots();

                pub fn new(sdk: SDK) -> Self {
                    let slot = fluentbase_sdk::U256::from(0);
                    let offset = 0u8;
                    #constructor
                }

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
            syn::Error::new_spanned(&input, "StorageLayout requires at least one field")
        })?;
        let first_field_name = first_field.ident.as_ref().unwrap();

        Ok(quote! {
            impl #name {
                pub const SLOTS: usize = Self::calculate_slots();

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

fn generate_constructor_body(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    has_sdk: bool,
) -> TokenStream2 {
    let mut layout_calculations = Vec::new();
    let mut field_assignments = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().expect("Named fields required");
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

// All other helper functions remain the same...
fn generate_layout_calculation(layout_var: &syn::Ident, field_type: &syn::Type) -> TokenStream2 {
    quote! {
        let #layout_var = {
            let bytes = <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES as u8;
            let slots = <#field_type as fluentbase_sdk::storage::StorageLayout>::SLOTS;

            let layout = if slots == 0 {
                // Packable type
                if current_offset + bytes <= 32 {
                    // Fits in current slot
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
                // Full slots type
                if current_offset > 0 {
                    current_slot = current_slot + fluentbase_sdk::U256::from(1);
                    current_offset = 0;
                }
                let result = (current_slot, 0);
                current_slot = current_slot + fluentbase_sdk::U256::from(slots);
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

fn generate_accessor_methods(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> TokenStream2 {
    let methods = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().expect("Named fields required");
        let field_type = &field.ty;
        let doc_string = format!("Returns an accessor for the {} field", field_name);
        let method_name = quote::format_ident!("{}_accessor", field_name);

        quote! {
            #[doc = #doc_string]
            #[inline]
            pub fn #method_name(&self) -> <#field_type as fluentbase_sdk::storage::StorageLayout>::Accessor {
                <#field_type as fluentbase_sdk::storage::StorageLayout>::access(self.#field_name)
            }
        }
    });

    quote! { #(#methods)* }
}

fn generate_const_slots_calculation(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> TokenStream2 {
    let field_calculations = fields.iter().map(|field| {
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
    });

    quote! {
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
    }
}

fn generate_const_bytes_calculation(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> TokenStream2 {
    let field_calculations = fields.iter().map(|field| {
        let field_type = &field.ty;
        quote! {
            total_bytes += <#field_type as fluentbase_sdk::storage::StorageLayout>::BYTES;
        }
    });

    quote! {
        const fn calculate_bytes() -> usize {
            let mut total_bytes: usize = 0;
            #(#field_calculations)*
            total_bytes
        }
    }
}

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
