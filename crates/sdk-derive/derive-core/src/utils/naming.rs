use convert_case::{Case, Casing};

/// Solidity spelling of a Rust method name.
///
/// An idiomatic snake_case name is converted to camelCase (`balance_of` → `balanceOf`). A name
/// that already contains an uppercase letter is taken verbatim: `mintNFT`, `tokenURI` and
/// `DOMAIN_SEPARATOR` have no snake_case spelling that converts back to them, and re-casing such
/// a name (`mintNFT` → `mintNft`) silently changes the selector the router dispatches on and the
/// client encodes.
#[must_use]
pub fn solidity_function_name(rust_name: &str) -> String {
    if rust_name.chars().any(|c| c.is_ascii_uppercase()) {
        rust_name.to_string()
    } else {
        rust_name.to_case(Case::Camel)
    }
}

#[cfg(test)]
mod tests {
    use super::solidity_function_name;

    #[test]
    fn snake_case_names_become_camel_case() {
        assert_eq!(solidity_function_name("balance_of"), "balanceOf");
        assert_eq!(solidity_function_name("transfer"), "transfer");
        assert_eq!(
            solidity_function_name("safe_transfer_from"),
            "safeTransferFrom"
        );
    }

    #[test]
    fn names_with_uppercase_letters_are_kept_verbatim() {
        assert_eq!(solidity_function_name("mintNFT"), "mintNFT");
        assert_eq!(solidity_function_name("tokenURI"), "tokenURI");
        assert_eq!(
            solidity_function_name("DOMAIN_SEPARATOR"),
            "DOMAIN_SEPARATOR"
        );
        assert_eq!(solidity_function_name("balanceOf"), "balanceOf");
    }
}
