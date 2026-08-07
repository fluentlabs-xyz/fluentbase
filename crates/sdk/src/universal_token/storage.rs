use crate::{
    storage::MapKey,
    universal_token::{
        command::{
            AllowanceCommand, ApproveCommand, BalanceOfCommand, BurnCommand, MintCommand,
            NoncesCommand, PermitCommand, TransferCommand, TransferFromCommand,
            UniversalTokenCommand, WithdrawCommand,
        },
        consts::{
            ALLOWANCE_STORAGE_SLOT, BALANCE_STORAGE_SLOT, CONTRACT_FROZEN_STORAGE_SLOT,
            DECIMALS_STORAGE_SLOT, MINTER_STORAGE_SLOT, NAME_STORAGE_SLOT, NONCES_STORAGE_SLOT,
            PAUSER_STORAGE_SLOT, SIG_ERC20_ALLOWANCE, SIG_ERC20_APPROVE, SIG_ERC20_BALANCE,
            SIG_ERC20_BALANCE_OF, SIG_ERC20_BURN, SIG_ERC20_DECIMALS, SIG_ERC20_DEPOSIT,
            SIG_ERC20_DOMAIN_SEPARATOR, SIG_ERC20_MINT, SIG_ERC20_NAME, SIG_ERC20_NONCES,
            SIG_ERC20_PAUSE, SIG_ERC20_PERMIT, SIG_ERC20_SYMBOL, SIG_ERC20_TOTAL_SUPPLY,
            SIG_ERC20_TRANSFER, SIG_ERC20_TRANSFER_FROM, SIG_ERC20_UNPAUSE, SIG_ERC20_WITHDRAW,
            SYMBOL_STORAGE_SLOT, TOTAL_SUPPLY_STORAGE_SLOT, WRAPPED_STORAGE_SLOT,
        },
    },
};
use alloc::vec::Vec;
use fluentbase_codec::{Codec, SolidityABI};
use fluentbase_types::{bytes::BytesMut, Address, Bytes, B256, U256, UNIVERSAL_TOKEN_MAGIC_BYTES};

pub const SIG_LEN_BYTES: usize = 4;

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy, Codec)]
#[repr(transparent)]
pub struct TokenNameOrSymbol {
    bytes: B256,
}

impl From<&str> for TokenNameOrSymbol {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl TokenNameOrSymbol {
    /// Wraps a raw 32-byte metadata word as it appears in calldata or storage.
    ///
    /// The bytes are not validated; use [`Self::as_str`] to decode them.
    pub const fn from_word(bytes: B256) -> Self {
        Self { bytes }
    }

    /// Encodes `value` as a 32-byte short string, or returns `None` if it does not fit.
    ///
    /// Prefer this over [`Self::from_str`]: it reports overlong metadata instead of
    /// silently shortening it to a different token name.
    pub fn try_from_str(value: &str) -> Option<Self> {
        if value.len() > B256::len_bytes() {
            return None;
        }
        let mut bytes = B256::ZERO;
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Some(Self { bytes })
    }

    /// Encodes `value` as a 32-byte short string, truncating anything longer.
    ///
    /// Truncation is lossy and silent — it behaves the same in debug and release, so it
    /// cannot be relied on to surface overlong metadata. Use [`Self::try_from_str`] when
    /// the caller needs to know the name did not fit.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        // Truncate on a char boundary. Slicing at a fixed 32 bytes can land inside a
        // multi-byte code point, and the resulting word decodes nowhere.
        let mut len = core::cmp::min(B256::len_bytes(), value.len());
        while len > 0 && !value.is_char_boundary(len) {
            len -= 1;
        }
        let mut bytes = B256::ZERO;
        bytes[..len].copy_from_slice(&value.as_bytes()[..len]);
        Self { bytes }
    }

    pub fn as_str(&self) -> Option<&str> {
        let length = self
            .bytes
            .iter()
            .position(|c| *c == 0u8)
            .unwrap_or(B256::len_bytes());
        str::from_utf8(&self.bytes[0..length]).ok()
    }
}

#[derive(Default, Debug, PartialEq, Codec)]
pub struct LegacyInitialSettings {
    pub token_name: [u8; 32],
    pub token_symbol: [u8; 32],
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
}

impl LegacyInitialSettings {
    /// Decodes a legacy payload, requiring the input to be exactly one payload long.
    ///
    /// The decoder itself only checks that the buffer is *at least* large enough, so an exact
    /// length check here is what stops a caller from accepting — and later persisting — bytes
    /// that carry no token semantics.
    pub fn decode_with_prefix(buf: &[u8]) -> Option<Self> {
        if buf.len() != INITIAL_SETTINGS_LEGACY_SIZE {
            return None;
        }
        let (sig, buf) = buf.split_at(4);
        if sig != UNIVERSAL_TOKEN_MAGIC_BYTES {
            return None;
        }
        let result: Self = SolidityABI::decode(&buf, 0).ok()?;
        Some(result)
    }
}

/// Initial settings payload sizes including magic prefix.
pub const INITIAL_SETTINGS_V1_SIZE: usize = 4 + 6 * 32;
pub const INITIAL_SETTINGS_V2_SIZE: usize = 4 + 7 * 32;

/// Legacy payload size including magic prefix.
///
/// The legacy layout stores `token_name`/`token_symbol` as `[u8; 32]`, and the Solidity encoder
/// gives every `u8` element its own word — hence 64 words of names against 4 words of everything
/// else. `test_legacy_size_matches_the_encoder` pins this constant to the encoder.
pub const INITIAL_SETTINGS_LEGACY_SIZE: usize = 4 + 68 * 32;

/// Largest payload any accepted creation form occupies, prefix included.
pub const INITIAL_SETTINGS_MAX_SIZE: usize = INITIAL_SETTINGS_LEGACY_SIZE;

#[derive(Default, Debug, PartialEq, Codec)]
struct InitialSettingsV1 {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
}

#[derive(Default, Debug, PartialEq, Codec)]
struct InitialSettingsV2 {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
    pub wrapped: bool,
}

#[derive(Default, Debug, PartialEq)]
pub struct InitialSettings {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
    /// Enables wrapped-token extension (`deposit()` / `withdraw(uint256)`).
    pub wrapped: Option<bool>,
}

impl InitialSettings {
    pub fn encode_with_prefix(&self) -> Bytes {
        let mut bytes = BytesMut::new();
        if let Some(wrapped) = self.wrapped {
            let settings = InitialSettingsV2 {
                token_name: self.token_name,
                token_symbol: self.token_symbol,
                decimals: self.decimals,
                initial_supply: self.initial_supply,
                minter: self.minter,
                pauser: self.pauser,
                wrapped,
            };
            SolidityABI::encode(&settings, &mut bytes, 0).unwrap();
        } else {
            let settings = InitialSettingsV1 {
                token_name: self.token_name,
                token_symbol: self.token_symbol,
                decimals: self.decimals,
                initial_supply: self.initial_supply,
                minter: self.minter,
                pauser: self.pauser,
            };
            SolidityABI::encode(&settings, &mut bytes, 0).unwrap();
        }
        let result = bytes.freeze();
        // TODO(d1r1): Optimize allocation
        let mut output = Vec::with_capacity(UNIVERSAL_TOKEN_MAGIC_BYTES.len() + result.len());
        output.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
        output.extend_from_slice(&result);
        output.into()
    }

    /// Decodes a creation payload, accepting only the exact canonical V1, V2 and legacy forms.
    ///
    /// Length is matched exactly rather than as a lower bound. The underlying Solidity decoder is
    /// happy to stop short of the end of its buffer, so a lower bound would let a creator append
    /// arbitrary bytes that decode to the same token — bytes the constructor would then persist as
    /// account metadata. Re-encoding the result via [`Self::encode_with_prefix`] is therefore
    /// always canonical and never longer than the input.
    pub fn decode_with_prefix(buf: &[u8]) -> Option<Self> {
        if buf.len() < 4 {
            return None;
        }
        let (sig, payload) = buf.split_at(4);
        if sig != UNIVERSAL_TOKEN_MAGIC_BYTES {
            return None;
        }

        match buf.len() {
            INITIAL_SETTINGS_V1_SIZE => {
                let settings: InitialSettingsV1 = SolidityABI::decode(&payload, 0).ok()?;
                Some(Self {
                    token_name: settings.token_name,
                    token_symbol: settings.token_symbol,
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: None,
                })
            }
            INITIAL_SETTINGS_V2_SIZE => {
                let settings: InitialSettingsV2 = SolidityABI::decode(&payload, 0).ok()?;
                Some(Self {
                    token_name: settings.token_name,
                    token_symbol: settings.token_symbol,
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: Some(settings.wrapped),
                })
            }
            INITIAL_SETTINGS_LEGACY_SIZE => {
                // Legacy format uses a different layout and larger payload.
                let settings = LegacyInitialSettings::decode_with_prefix(buf)?;
                Some(Self {
                    token_name: TokenNameOrSymbol {
                        bytes: settings.token_name.into(),
                    },
                    token_symbol: TokenNameOrSymbol {
                        bytes: settings.token_symbol.into(),
                    },
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: None,
                })
            }
            _ => None,
        }
    }
}

pub fn erc20_compute_deploy_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(8);
    let InitialSettings {
        minter,
        pauser,
        initial_supply,
        wrapped,
        ..
    } = InitialSettings::decode_with_prefix(input)?;
    result.push(DECIMALS_STORAGE_SLOT);
    result.push(NAME_STORAGE_SLOT);
    result.push(SYMBOL_STORAGE_SLOT);
    result.push(TOTAL_SUPPLY_STORAGE_SLOT);
    if !initial_supply.is_zero() {
        let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
        result.push(storage_slot);
    }
    if !minter.is_zero() {
        result.push(MINTER_STORAGE_SLOT);
    }
    if !pauser.is_zero() {
        result.push(PAUSER_STORAGE_SLOT);
    }
    // Push wrapped flag only if we use V2 settings
    if wrapped.is_some() {
        result.push(WRAPPED_STORAGE_SLOT);
    }
    Some(result)
}

pub fn erc20_compute_main_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(8);
    let (sig, input) = input.split_at(SIG_LEN_BYTES);
    let sig = u32::from_be_bytes(sig.try_into().unwrap());
    match sig {
        SIG_ERC20_TOTAL_SUPPLY => result.push(TOTAL_SUPPLY_STORAGE_SLOT),
        SIG_ERC20_TRANSFER => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
            let TransferCommand { to, .. } = TransferCommand::try_decode(input).ok()?;
            let storage_slot = to.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
        }
        SIG_ERC20_TRANSFER_FROM => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let TransferFromCommand { from, to, .. } =
                TransferFromCommand::try_decode(input).ok()?;
            result.push(from.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(to.compute_slot(BALANCE_STORAGE_SLOT));
            let allowance_slot = from.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(caller.compute_slot(allowance_slot));
        }
        SIG_ERC20_BALANCE => {
            let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
        }
        SIG_ERC20_BALANCE_OF => {
            let BalanceOfCommand { owner } = BalanceOfCommand::try_decode(input).ok()?;
            result.push(owner.compute_slot(BALANCE_STORAGE_SLOT));
        }
        SIG_ERC20_SYMBOL => result.push(SYMBOL_STORAGE_SLOT),
        SIG_ERC20_NAME => result.push(NAME_STORAGE_SLOT),
        SIG_ERC20_DECIMALS => result.push(DECIMALS_STORAGE_SLOT),
        SIG_ERC20_ALLOWANCE => {
            let AllowanceCommand { owner, spender } = AllowanceCommand::try_decode(input).ok()?;
            let allowance_slot = owner.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(spender.compute_slot(allowance_slot));
        }
        SIG_ERC20_APPROVE => {
            let ApproveCommand { spender, .. } = ApproveCommand::try_decode(input).ok()?;
            let allowance_slot = caller.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(spender.compute_slot(allowance_slot));
        }
        SIG_ERC20_PERMIT => {
            let PermitCommand { owner, spender, .. } = PermitCommand::try_decode(input).ok()?;
            let allowance_slot = owner.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(spender.compute_slot(allowance_slot));
            result.push(owner.compute_slot(NONCES_STORAGE_SLOT));
            result.push(NAME_STORAGE_SLOT);
        }
        SIG_ERC20_NONCES => {
            let NoncesCommand { owner } = NoncesCommand::try_decode(input).ok()?;
            result.push(owner.compute_slot(NONCES_STORAGE_SLOT));
        }
        SIG_ERC20_DOMAIN_SEPARATOR => {
            result.push(NAME_STORAGE_SLOT);
        }
        SIG_ERC20_MINT => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let MintCommand { to, .. } = MintCommand::try_decode(input).ok()?;
            result.push(to.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(MINTER_STORAGE_SLOT);
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_BURN => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let BurnCommand { from, .. } = BurnCommand::try_decode(input).ok()?;
            result.push(from.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(MINTER_STORAGE_SLOT);
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_PAUSE => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(PAUSER_STORAGE_SLOT);
        }
        SIG_ERC20_UNPAUSE => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(PAUSER_STORAGE_SLOT);
        }
        SIG_ERC20_DEPOSIT => {
            result.push(WRAPPED_STORAGE_SLOT);
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(caller.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_WITHDRAW => {
            result.push(WRAPPED_STORAGE_SLOT);
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let WithdrawCommand { .. } = WithdrawCommand::try_decode(input).ok()?;
            result.push(caller.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        _ => {}
    }
    Some(result)
}

pub fn erc20_compute_storage_keys(
    input: &[u8],
    caller: &Address,
    is_create: bool,
) -> Option<Vec<U256>> {
    if is_create {
        erc20_compute_deploy_storage_keys(input, caller)
    } else {
        erc20_compute_main_storage_keys(input, caller)
    }
}

#[cfg(test)]
mod tests {
    use crate::universal_token::storage::{
        InitialSettings, LegacyInitialSettings, TokenNameOrSymbol, INITIAL_SETTINGS_LEGACY_SIZE,
        INITIAL_SETTINGS_V1_SIZE, INITIAL_SETTINGS_V2_SIZE,
    };
    use alloc::{format, vec::Vec};
    use fluentbase_codec::SolidityABI;
    use fluentbase_types::{
        address, bytes::BytesMut, Address, Bytes, B256, U256, UNIVERSAL_TOKEN_MAGIC_BYTES,
    };

    #[test]
    fn test_token_name_boundaries() {
        // 32 bytes is the inclusive limit, for ASCII and multibyte alike.
        let ascii = "a".repeat(32);
        assert_eq!(
            TokenNameOrSymbol::try_from_str(&ascii).unwrap().as_str(),
            Some(ascii.as_str())
        );
        let multibyte = "😀".repeat(8);
        assert_eq!(
            TokenNameOrSymbol::try_from_str(&multibyte)
                .unwrap()
                .as_str(),
            Some(multibyte.as_str())
        );

        // Overlong input is rejected rather than reshaped into a different token.
        assert!(TokenNameOrSymbol::try_from_str(&"a".repeat(33)).is_none());
        assert!(TokenNameOrSymbol::try_from_str(&format!("{}é", "a".repeat(31))).is_none());
    }

    #[test]
    fn test_token_name_truncation_stays_decodable() {
        // The infallible constructor still truncates, but never mid-code-point: a 33-byte
        // input whose 32-byte prefix splits `é` must drop the whole character.
        let split = format!("{}é", "a".repeat(31));
        assert_eq!(split.len(), 33);
        assert_eq!(
            TokenNameOrSymbol::from_str(&split).as_str(),
            Some("a".repeat(31).as_str())
        );
        assert_eq!(
            TokenNameOrSymbol::from_str(&"😀".repeat(9)).as_str(),
            Some("😀".repeat(8).as_str())
        );
    }

    #[test]
    fn test_token_name_from_word_rejects_malformed() {
        // Raw words arrive from calldata and legacy layouts unvalidated.
        let mut word = B256::ZERO;
        word[0] = 0x80; // lone continuation byte
        assert_eq!(TokenNameOrSymbol::from_word(word).as_str(), None);
    }

    #[test]
    fn test_ops_u256_overflow() {
        let mut value = U256::from(0);
        value.set_bit(0, true);
        value.set_bit(1, true);
        value.set_bit(8, true);
        assert_eq!(&value.as_le_slice()[0..3], &[3, 1, 0]);
        unsafe {
            value.as_le_slice_mut()[1] = 22;
        }
        assert_eq!(&value.as_le_slice()[0..3], &[3, 22, 0]);
    }

    #[test]
    fn test_ser_der() {
        let settings = InitialSettings {
            token_name: TokenNameOrSymbol::from_str("Hello"),
            token_symbol: TokenNameOrSymbol::from_str("World"),
            decimals: 12,
            initial_supply: U256::from(2),
            minter: address!("0303000200500020400000040000002000809020"),
            pauser: Address::ZERO,
            wrapped: None,
        };
        let addr = address!("0003000200500000400000040000002000800020");
        let addr_bytes: [u8; Address::len_bytes()] = addr.into();
        let addr_restored: Address = addr_bytes.into();
        assert_eq!(addr, addr_restored);
        let settings_vec = settings.encode_with_prefix();
        assert_eq!(settings_vec.len(), INITIAL_SETTINGS_V1_SIZE);
        let settings_restored = InitialSettings::decode_with_prefix(settings_vec.as_ref()).unwrap();
        assert_eq!(settings, settings_restored);
    }

    #[test]
    fn test_ser_der_wrapped_settings() {
        let settings = InitialSettings {
            token_name: TokenNameOrSymbol::from_str("WETH"),
            token_symbol: TokenNameOrSymbol::from_str("WETH"),
            decimals: 18,
            initial_supply: U256::ZERO,
            minter: Address::ZERO,
            pauser: Address::ZERO,
            wrapped: Some(true),
        };

        let settings_vec = settings.encode_with_prefix();
        assert_eq!(settings_vec.len(), INITIAL_SETTINGS_V2_SIZE);
        let settings_restored = InitialSettings::decode_with_prefix(settings_vec.as_ref()).unwrap();
        assert_eq!(settings, settings_restored);
    }

    /// Builds a canonical legacy payload for `name`/`symbol` short strings.
    fn legacy_payload(name: &str, symbol: &str) -> Bytes {
        let mut token_name = [0u8; 32];
        token_name[..name.len()].copy_from_slice(name.as_bytes());
        let mut token_symbol = [0u8; 32];
        token_symbol[..symbol.len()].copy_from_slice(symbol.as_bytes());

        let settings = LegacyInitialSettings {
            token_name,
            token_symbol,
            decimals: 6,
            initial_supply: U256::from(7),
            minter: address!("0303000200500020400000040000002000809020"),
            pauser: address!("0003000200500000400000040000002000800020"),
        };
        let mut payload = BytesMut::new();
        SolidityABI::encode(&settings, &mut payload, 0).unwrap();

        let mut out = Vec::with_capacity(UNIVERSAL_TOKEN_MAGIC_BYTES.len() + payload.len());
        out.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
        out.extend_from_slice(&payload);
        out.into()
    }

    #[test]
    fn test_legacy_size_matches_the_encoder() {
        // Pins `INITIAL_SETTINGS_LEGACY_SIZE` to what the encoder actually produces, so the exact
        // length check cannot silently start rejecting every legacy payload.
        assert_eq!(
            legacy_payload("Legacy", "LGC").len(),
            INITIAL_SETTINGS_LEGACY_SIZE
        );
    }

    #[test]
    fn test_legacy_payload_decodes_to_v1_settings() {
        let decoded =
            InitialSettings::decode_with_prefix(&legacy_payload("Legacy", "LGC")).unwrap();
        assert_eq!(decoded.token_name.as_str(), Some("Legacy"));
        assert_eq!(decoded.token_symbol.as_str(), Some("LGC"));
        assert_eq!(decoded.decimals, 6);
        assert_eq!(decoded.initial_supply, U256::from(7));
        assert_eq!(decoded.wrapped, None);

        // Legacy carries no wrapped flag, so its canonical form is V1 — an order of magnitude
        // smaller than the payload it came from.
        assert_eq!(decoded.encode_with_prefix().len(), INITIAL_SETTINGS_V1_SIZE);
    }

    fn is_canonical_len(len: usize) -> bool {
        matches!(
            len,
            INITIAL_SETTINGS_V1_SIZE | INITIAL_SETTINGS_V2_SIZE | INITIAL_SETTINGS_LEGACY_SIZE
        )
    }

    /// The three payload forms that must decode, and nothing else.
    fn canonical_payloads() -> Vec<(&'static str, Bytes)> {
        let v1 = InitialSettings {
            token_name: TokenNameOrSymbol::from_str("Hello"),
            token_symbol: TokenNameOrSymbol::from_str("HLO"),
            decimals: 12,
            initial_supply: U256::from(2),
            minter: address!("0303000200500020400000040000002000809020"),
            pauser: Address::ZERO,
            wrapped: None,
        };
        let v2 = InitialSettings {
            wrapped: Some(true),
            minter: Address::ZERO,
            ..InitialSettings::default()
        };
        alloc::vec![
            ("v1", v1.encode_with_prefix()),
            ("v2", v2.encode_with_prefix()),
            ("legacy", legacy_payload("Legacy", "LGC")),
        ]
    }

    #[test]
    fn test_canonical_payloads_decode() {
        for (label, payload) in canonical_payloads() {
            assert!(
                InitialSettings::decode_with_prefix(&payload).is_some(),
                "{label} payload must decode"
            );
        }
    }

    #[test]
    fn test_trailing_bytes_are_rejected() {
        for (label, payload) in canonical_payloads() {
            for suffix_len in [1usize, 2, 31, 32, 33, 1024] {
                // A V1 payload plus exactly one word *is* a canonical V2 payload; that is a
                // different form, not an ignored suffix.
                if is_canonical_len(payload.len() + suffix_len) {
                    continue;
                }
                let mut padded = payload.to_vec();
                padded.extend(core::iter::repeat_n(0u8, suffix_len));
                assert!(
                    InitialSettings::decode_with_prefix(&padded).is_none(),
                    "{label} payload with {suffix_len} trailing bytes must be rejected"
                );
            }
        }
    }

    #[test]
    fn test_truncated_payloads_are_rejected() {
        for (label, payload) in canonical_payloads() {
            let truncated = &payload[..payload.len() - 1];
            assert!(
                InitialSettings::decode_with_prefix(truncated).is_none(),
                "{label} payload missing its last byte must be rejected"
            );
        }
    }

    #[test]
    fn test_only_canonical_lengths_are_accepted() {
        // Sweep every length up to and past the legacy form: the accepted set is exactly the
        // three canonical sizes, so no length can carry an ignored suffix.
        let template = legacy_payload("Legacy", "LGC");
        for len in 0..=INITIAL_SETTINGS_LEGACY_SIZE + 64 {
            let mut payload = Vec::with_capacity(len);
            payload.extend_from_slice(&template[..len.min(template.len())]);
            payload.resize(len, 0);

            let accepted = InitialSettings::decode_with_prefix(&payload).is_some();
            let canonical = is_canonical_len(len);
            // Only the legacy length is a real payload here; the shorter canonical lengths are
            // truncations of it, so they may or may not decode. What must hold is that no
            // non-canonical length ever decodes.
            assert!(
                !accepted || canonical,
                "length {len} is not canonical but decoded"
            );
        }
    }

    #[test]
    fn test_reencoding_a_decoded_payload_is_canonical() {
        for (label, payload) in canonical_payloads() {
            let decoded = InitialSettings::decode_with_prefix(&payload).unwrap();
            let reencoded = decoded.encode_with_prefix();

            assert!(
                reencoded.len() <= payload.len(),
                "{label}: canonical form must never grow"
            );
            // Re-encoding is a fixed point: decoding the canonical bytes yields the same settings.
            assert_eq!(
                InitialSettings::decode_with_prefix(&reencoded).unwrap(),
                decoded,
                "{label}: canonical form must round-trip"
            );
            if label != "legacy" {
                assert_eq!(
                    reencoded, payload,
                    "{label}: already-canonical input must be preserved byte for byte"
                );
            }
        }
    }
}
