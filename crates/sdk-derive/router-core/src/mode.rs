use crate::error::RouterError;
use core::str::FromStr;
use std::fmt;
use tracing::warn;

/// List of supported router modes
const VALID_MODES: &[&str] = &["solidity", "fluent"];

/// Represents the routing mode for the API implementation.
/// Different modes affect how the router handles method calls and interfaces.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RouterMode {
    /// Solidity-compatible mode for Ethereum ABI compatibility
    Solidity,
    /// Fluent API mode for idiomatic Rust interfaces
    Fluent,
}

impl RouterMode {
    /// Returns true if the provided string represents a valid router mode (case-insensitive).
    ///
    /// # Arguments
    /// * `s` - String to validate
    pub fn is_valid_str(s: &str) -> bool {
        VALID_MODES.contains(&s.to_lowercase().as_str())
    }

    /// Returns a slice containing all valid mode strings.
    ///
    /// # Returns
    /// A static slice of valid mode strings.
    pub fn valid_modes() -> &'static [&'static str] {
        VALID_MODES
    }

    /// Returns the default router mode.
    ///
    /// # Returns
    /// The default `RouterMode::Solidity` mode.
    pub const fn default_mode() -> Self {
        Self::Solidity
    }

    /// Returns true if the mode is set to Solidity.
    pub const fn is_solidity(&self) -> bool {
        matches!(self, Self::Solidity)
    }

    /// Returns true if the mode is set to Fluent.
    pub const fn is_fluent(&self) -> bool {
        matches!(self, Self::Fluent)
    }

    /// Attempts to create a RouterMode from a string with case-insensitive matching.
    ///
    /// # Arguments
    /// * `s` - String to parse into a RouterMode
    ///
    /// # Returns
    /// * `Ok(RouterMode)` - If the string matches a valid mode
    /// * `Err(RouterError)` - If the string is not a valid mode
    ///
    /// # Examples
    /// ```
    /// use router_core::mode::RouterMode;
    /// assert!(RouterMode::try_from_str("SOLIDITY").is_ok());
    /// assert!(RouterMode::try_from_str("solidity").is_ok());
    /// assert!(RouterMode::try_from_str("invalid").is_err());
    /// ```
    pub fn try_from_str(s: &str) -> Result<Self, RouterError> {
        let mode = s.to_lowercase();
        match mode.as_str() {
            "solidity" => Ok(Self::Solidity),
            "fluent" => Ok(Self::Fluent),
            invalid => {
                warn!("Attempted to use invalid router mode: {}", invalid);
                Err(RouterError::InvalidMode(format!(
                    "Invalid mode '{}'. Valid modes are: {}",
                    invalid,
                    VALID_MODES.join(", ")
                )))
            }
        }
    }
}

impl Default for RouterMode {
    fn default() -> Self {
        Self::default_mode()
    }
}

impl FromStr for RouterMode {
    type Err = RouterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl fmt::Display for RouterMode {
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
            assert!(matches!(
                valid_input.parse::<RouterMode>(),
                Ok(RouterMode::Solidity)
            ));
        }

        for valid_input in ["fluent", "FLUENT", "FluENT", "Fluent"] {
            assert!(matches!(
                valid_input.parse::<RouterMode>(),
                Ok(RouterMode::Fluent)
            ));
        }

        for invalid_input in ["invalid", "INVALID"] {
            assert!(invalid_input.parse::<RouterMode>().is_err());
        }
    }

    #[test]
    fn test_router_mode_is_valid_str() {
        for valid_input in [
            "solidity", "SOLIDITY", "SoLiDiTy", "Solidity", "fluent", "FLUENT", "FluENT", "Fluent",
        ] {
            assert!(RouterMode::is_valid_str(valid_input));
        }

        for invalid_input in ["invalid", "INVALID"] {
            assert!(!RouterMode::is_valid_str(invalid_input));
        }
    }

    #[test]
    fn test_router_mode_case_insensitive_comparison() {
        let test_cases = [
            ("solidity", true, Some(RouterMode::Solidity)),
            ("SOLIDITY", true, Some(RouterMode::Solidity)),
            ("SoLiDiTy", true, Some(RouterMode::Solidity)),
            ("fluent", true, Some(RouterMode::Fluent)),
            ("FLUENT", true, Some(RouterMode::Fluent)),
            ("FluENT", true, Some(RouterMode::Fluent)),
            ("invalid", false, None),
            ("INVALID", false, None),
        ];

        for (input, is_valid, expected_mode) in test_cases {
            assert_eq!(RouterMode::is_valid_str(input), is_valid);
            if let Some(expected) = expected_mode {
                assert_eq!(RouterMode::try_from_str(input).unwrap(), expected);
            } else {
                assert!(RouterMode::try_from_str(input).is_err());
            }
        }
    }

    #[test]
    fn test_mode_display_and_type_checks() {
        let solidity_mode = RouterMode::Solidity;
        assert_eq!(solidity_mode.to_string(), "solidity");
        assert!(solidity_mode.is_solidity());
        assert!(!solidity_mode.is_fluent());

        let fluent_mode = RouterMode::Fluent;
        assert_eq!(fluent_mode.to_string(), "fluent");
        assert!(fluent_mode.is_fluent());
        assert!(!fluent_mode.is_solidity());
    }

    #[test]
    fn test_mode_defaults_and_valid_modes() {
        assert_eq!(RouterMode::default(), RouterMode::Solidity);
        assert_eq!(RouterMode::default_mode(), RouterMode::Solidity);

        let modes = RouterMode::valid_modes();
        assert!(modes.contains(&"solidity"));
        assert!(modes.contains(&"fluent"));
        assert_eq!(modes.len(), 2);
    }
}
