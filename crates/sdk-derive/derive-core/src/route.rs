use crate::{
    abi::FunctionABI,
    codec::CodecGenerator,
    function_id::{create_function_id_mismatch_error, FunctionIDAttribute},
    mode::Mode,
};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use syn::{spanned::Spanned, Error, ImplItemFn, Index, Result};

#[derive(Debug)]
pub struct Route {
    /// Core ABI representation
    abi: FunctionABI,
    /// Original function implementation
    method: ImplItemFn,
    /// Validated function ID
    function_id: [u8; 4],
}

impl Route {
    pub fn new(method: ImplItemFn) -> Result<Self> {
        // Convert method to ABI representation
        let abi = FunctionABI::from_impl_fn(&method).map_err(|e| {
            Error::new(
                method.span(),
                format!("Failed to convert method to ABI: {}", e),
            )
        })?;

        // Get canonical signature
        let signature = abi.signature();
        // Calculate selector
        let calculated_id = abi.selector();

        // Validate against attribute if present
        let function_id = if let Some(attr) = method
            .attrs
            .iter()
            .find(|a| a.path().is_ident("function_id"))
        {
            let attr_value = attr.parse_args::<FunctionIDAttribute>()?;
            let attr_id = attr_value.function_id_bytes()?;

            if attr_value.validate.unwrap_or(true) && attr_id != calculated_id {
                return Err(create_function_id_mismatch_error(
                    attr.span(),
                    &calculated_id,
                    &attr_id,
                    signature,
                ));
            }
            attr_id
        } else {
            calculated_id
        };

        Ok(Self {
            abi,
            method,
            function_id,
        })
    }

    /// Returns reference to the ABI representation
    pub fn abi(&self) -> &FunctionABI {
        &self.abi
    }

    /// Returns reference to the original method
    pub fn method(&self) -> &ImplItemFn {
        &self.method
    }

    pub fn generate_codec_impl(&self, mode: &Mode) -> TokenStream2 {
        let codec_generator = CodecGenerator::new(&self.abi, &self.function_id, mode);

        codec_generator
            .generate()
            .expect("failed to generate codec")
    }
}

impl ToTokens for Route {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        // Generate dispatch logic based on ABI
        let dispatch = self.generate_match_arm();
        tokens.extend(dispatch);
    }
}

// Private implementation details
impl Route {
    /// Generates a match arm for the router's match expression.
    ///
    /// This function produces code that:
    /// 1. Decodes function parameters from input data
    /// 2. Calls the actual contract method
    /// 3. Encodes and writes the result
    ///
    /// # Generated code structure
    /// ```ignore
    /// SomeFunctionCall::SELECTOR => {
    ///     // Decode parameters from input
    ///     let (param1, param2) = match SomeFunctionCall::decode(&params) {
    ///         Ok(decoded) => (decoded.0.0, decoded.0.1),
    ///         Err(err) => panic!("Failed to decode parameters: {:?}", err),
    ///     };
    ///
    ///     // Call method and encode result
    ///     let output = self.some_function(&param1, &mut param2);
    ///     let encoded = SomeReturn::new(output).encode();
    ///     self.sdk.write(&encoded);
    /// }
    /// ```
    fn generate_match_arm(&self) -> TokenStream2 {
        // Get function name from implementation
        let fn_name = &self.method.sig.ident;

        // Generate decoder/encoder type names based on ABI name
        let call_type = format_ident!("{}Call", &self.abi.name.to_case(Case::Pascal));
        let return_type = format_ident!("{}Return", &self.abi.name.to_case(Case::Pascal));

        // Generate parameter tokens
        let declarations: Vec<_> = self
            .abi
            .inputs
            .iter()
            .map(|p| p.to_declaration_tokens())
            .collect();
        let arguments: Vec<_> = self
            .abi
            .inputs
            .iter()
            .map(|p| p.to_argument_tokens())
            .collect();
        let param_indices: Vec<Index> = (0..self.abi.inputs.len()).map(Index::from).collect();

        // Generate the result handling code based on number of outputs
        let output_handling = match self.abi.outputs.len() {
            0 => quote! {
                self.#fn_name(#(#arguments),*);
                self.sdk.write(&[0u8; 0]);
            },
            1 => quote! {
                let output = self.#fn_name(#(#arguments),*);
                let encoded_output = #return_type::new((output,)).encode();
                self.sdk.write(&encoded_output);
            },
            _ => quote! {
                let output = self.#fn_name(#(#arguments),*);
                let encoded_output = #return_type::new(output).encode();
                self.sdk.write(&encoded_output);
            },
        };

        // Combine into final match arm
        quote! {
            #call_type::SELECTOR => {
                let (#(#declarations),*) = match #call_type::decode(&params) {
                    Ok(decoded) => (#(decoded.0.#param_indices),*),
                    Err(err) => panic!("Failed to decode input parameters"),
                };

                #output_handling
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_route_new_success() {
        let method: ImplItemFn = parse_quote! {
            #[function_id("0xd3c914ca")]
            fn test_function(arg1: u32, arg2: String) -> bool {
                true
            }
        };

        let route = Route::new(method);
        assert!(route.is_ok());
        let route = route.unwrap();

        assert_eq!(route.abi.selector(), [0xd3, 0xc9, 0x14, 0xca]);
        assert_eq!(route.abi.signature(), "testFunction(uint32,string)");
        assert_eq!(route.abi.inputs.len(), 2);
        assert_eq!(route.abi.outputs.len(), 1);
    }
}
