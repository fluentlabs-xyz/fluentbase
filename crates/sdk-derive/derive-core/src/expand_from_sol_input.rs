use crate::abi::{
    error::ABIError,
    types::{convert_solidity_type, sol_to_rust},
};
use alloy_sol_macro_input::{SolInput, SolInputKind};
use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;
use syn_solidity::{
    visit::{visit_file, Visit},
    File,
    Item,
    ItemFunction,
    Mutability,
    Spanned,
    VariableDeclaration,
};
pub struct FunctionCollector<'a> {
    pub functions: Vec<&'a ItemFunction>,
}

impl<'a> Visit<'a> for FunctionCollector<'a> {
    fn visit_item_function(&mut self, func: &'a ItemFunction) {
        self.functions.push(func);
    }
}

/// Expand the input from Solidity file or JSON ABI to a Rust trait.
pub fn expand_from_sol_input(input: SolInput) -> syn::Result<TokenStream> {
    let file = match input.kind {
        SolInputKind::Sol(sol_file) => sol_file,
        SolInputKind::Type(_) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Expected Solidity interface or contract, not type",
            ));
        }
        #[cfg(feature = "json")]
        SolInputKind::Json(_, _) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "JSON ABI not supported in this macro",
            ));
        }
    };

    let mut visitor = FunctionCollector { functions: vec![] };
    visit_file(&mut visitor, &file);

    let mut trait_fns = Vec::new();
    for func in visitor.functions {
        let tokens = generate_trait_function(func)?;
        trait_fns.push(tokens);
    }

    let trait_name = resolve_trait_name(&file);

    Ok(quote! {
        pub trait #trait_name {
            #(#trait_fns)*
        }
    })
}

pub fn resolve_trait_name(file: &File) -> Ident {
    for item in &file.items {
        if let Item::Contract(contract) = item {
            if contract.kind.is_interface() {
                return format_ident!("{}", contract.name.to_string().to_case(Case::Pascal));
            } else if contract.kind.is_contract() {
                return format_ident!("I{}", contract.name.to_string().to_case(Case::Pascal));
            }
        }
    }

    format_ident!("GeneratedSolidityTrait")
}

pub fn resolve_receiver(func: &ItemFunction) -> TokenStream {
    if let Some(Mutability::View(_)) | Some(Mutability::Pure(_)) = func.attributes.mutability() {
        quote! { &self }
    } else {
        quote! { &mut self }
    }
}

pub fn generate_trait_function(func: &ItemFunction) -> syn::Result<TokenStream> {
    let Some(name) = &func.name else {
        return Ok(quote! {});
    };
    if name == "fallback" || name == "receive" {
        return Ok(quote! {});
    }

    let fn_name = format_ident!("{}", name.to_string().to_case(Case::Snake));
    let receiver = resolve_receiver(func);

    let mut args = Vec::new();
    for (i, param) in func.parameters.iter().enumerate() {
        args.push(generate_fn_param(i, param)?);
    }

    let ret = generate_fn_return(func)?;

    Ok(quote! {
        fn #fn_name(#receiver #(, #args)*) #ret;
    })
}

pub fn generate_fn_param(index: usize, param: &VariableDeclaration) -> syn::Result<TokenStream> {
    let name_str = param
        .name
        .as_ref()
        .map(|n| n.to_string().to_case(Case::Snake))
        .unwrap_or_else(|| format!("_param{index}"));
    let name_ident = format_ident!("{}", name_str);

    let sol_ty = convert_solidity_type(&param.ty)?;
    let rust_ty = sol_to_rust(&sol_ty).map_err(|e: ABIError| {
        syn::Error::new(param.ty.span(), format!("Cannot convert param type: {e}"))
    })?;

    Ok(quote! { #name_ident: #rust_ty })
}

pub fn generate_fn_return(func: &ItemFunction) -> syn::Result<TokenStream> {
    let Some(returns) = &func.returns else {
        return Ok(quote! {});
    };

    let return_params = &returns.returns;

    if return_params.len() == 1 {
        let sol_ty = convert_solidity_type(&return_params[0].ty)?;
        let rust_ty = sol_to_rust(&sol_ty).map_err(|e: ABIError| {
            syn::Error::new(
                return_params[0].ty.span(),
                format!("Return type error: {e}"),
            )
        })?;
        Ok(quote! { -> #rust_ty })
    } else {
        let rust_types: Vec<_> = return_params
            .iter()
            .map(|param| {
                let sol_ty = convert_solidity_type(&param.ty)?;
                sol_to_rust(&sol_ty).map(|ty| quote! { #ty }).map_err(|e| {
                    syn::Error::new(param.ty.span(), format!("Tuple return type error: {e}"))
                })
            })
            .collect::<syn::Result<_>>()?;

        Ok(quote! { -> (#(#rust_types),*) })
    }
}
