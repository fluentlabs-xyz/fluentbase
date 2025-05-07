use darling::FromMeta;
use proc_macro2::Span;
use std::{fmt, str::FromStr};
use syn;
use tracing::warn;

/// List of supported router modes
const VALID_MODES: &[&str] = &["solidity", "fluent"];

/// Represents the routing mode for the API implementation.
/// Different modes affect how the router handles method calls and interfaces.
#[derive(Debug, PartialEq, Clone, Copy, FromMeta)]
pub enum Mode {
    /// Solidity-compatible mode for Ethereum ABI compatibility
    Solidity,
    /// Fluent API mode for idiomatic Rust interfaces
    Fluent,
}

impl Mode {
    /// Returns true if the provided string represents a valid router mode (case-insensitive).
    ///
    /// # Arguments
    /// * `s` - String to validate
    #[must_use]
    pub fn is_valid_str(s: &str) -> bool {
        VALID_MODES.contains(&s.to_lowercase().as_str())
    }

    /// Returns a slice containing all valid mode strings.
    ///
    /// # Returns
    /// A static slice of valid mode strings.
    #[must_use]
    pub fn valid_modes() -> &'static [&'static str] {
        VALID_MODES
    }

    /// Returns the default router mode.
    ///
    /// # Returns
    /// The default `Mode::Solidity` mode.
    #[must_use]
    pub const fn default_mode() -> Self {
        Self::Solidity
    }

    /// Returns true if the mode is set to Solidity.
    #[must_use]
    pub const fn is_solidity(&self) -> bool {
        matches!(self, Self::Solidity)
    }

    /// Returns true if the mode is set to Fluent.
    #[must_use]
    pub const fn is_fluent(&self) -> bool {
        matches!(self, Self::Fluent)
    }

    /// Attempts to create a `Mode` from a string with case-insensitive matching.
    ///
    /// # Arguments
    /// * `s` - String to parse into a `Mode`
    ///
    /// # Returns
    /// * `Ok(Mode)` - If the string matches a valid mode
    /// * `Err(syn::Error)` - If the string is not a valid mode
    ///
    /// # Examples
    /// ```
    /// use fluentbase_sdk_derive_core::mode::Mode;
    /// assert!(Mode::try_from_str("SOLIDITY").is_ok());
    /// assert!(Mode::try_from_str("solidity").is_ok());
    /// assert!(Mode::try_from_str("invalid").is_err());
    /// ```
    pub fn try_from_str(s: &str) -> Result<Self, syn::Error> {
        let mode = s.to_lowercase();
        match mode.as_str() {
            "solidity" => Ok(Self::Solidity),
            "fluent" => Ok(Self::Fluent),
            invalid => {
                warn!("Attempted to use invalid router mode: {}", invalid);
                Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "Invalid mode '{}'. Valid modes are: {}",
                        invalid,
                        VALID_MODES.join(", ")
                    ),
                ))
            }
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Self::default_mode()
    }
}

impl FromStr for Mode {
    type Err = syn::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solidity => write!(f, "solidity"),
            Self::Fluent => write!(f, "fluent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_mode_from_str() {
        for valid_input in ["solidity", "SOLIDITY", "SoLiDiTy", "Solidity"] {
            assert!(matches!(valid_input.parse::<Mode>(), Ok(Mode::Solidity)));
        }

        for valid_input in ["fluent", "FLUENT", "FluENT", "Fluent"] {
            assert!(matches!(valid_input.parse::<Mode>(), Ok(Mode::Fluent)));
        }

        for invalid_input in ["invalid", "INVALID"] {
            assert!(invalid_input.parse::<Mode>().is_err());
        }
    }

    #[test]
    fn test_router_mode_is_valid_str() {
        for valid_input in [
            "solidity", "SOLIDITY", "SoLiDiTy", "Solidity", "fluent", "FLUENT", "FluENT", "Fluent",
        ] {
            assert!(Mode::is_valid_str(valid_input));
        }

        for invalid_input in ["invalid", "INVALID"] {
            assert!(!Mode::is_valid_str(invalid_input));
        }
    }

    #[test]
    fn test_router_mode_case_insensitive_comparison() {
        let test_cases = [
            ("solidity", true, Some(Mode::Solidity)),
            ("SOLIDITY", true, Some(Mode::Solidity)),
            ("SoLiDiTy", true, Some(Mode::Solidity)),
            ("fluent", true, Some(Mode::Fluent)),
            ("FLUENT", true, Some(Mode::Fluent)),
            ("FluENT", true, Some(Mode::Fluent)),
            ("invalid", false, None),
            ("INVALID", false, None),
        ];

        for (input, is_valid, expected_mode) in test_cases {
            assert_eq!(Mode::is_valid_str(input), is_valid);
            if let Some(expected) = expected_mode {
                assert_eq!(Mode::try_from_str(input).unwrap(), expected);
            } else {
                assert!(Mode::try_from_str(input).is_err());
            }
        }
    }

    #[test]
    fn test_mode_display_and_type_checks() {
        let solidity_mode = Mode::Solidity;
        assert_eq!(solidity_mode.to_string(), "solidity");
        assert!(solidity_mode.is_solidity());
        assert!(!solidity_mode.is_fluent());

        let fluent_mode = Mode::Fluent;
        assert_eq!(fluent_mode.to_string(), "fluent");
        assert!(fluent_mode.is_fluent());
        assert!(!fluent_mode.is_solidity());
    }

    #[test]
    fn test_mode_defaults_and_valid_modes() {
        assert_eq!(Mode::default(), Mode::Solidity);
        assert_eq!(Mode::default_mode(), Mode::Solidity);

        let modes = Mode::valid_modes();
        assert!(modes.contains(&"solidity"));
        assert!(modes.contains(&"fluent"));
        assert_eq!(modes.len(), 2);
    }
}
