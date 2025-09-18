use serde::{Deserialize, Serialize};
use {
    crate::token_2022::extension::{Extension, ExtensionType},
    crate::token_2022::spl_pod::optional_keys::OptionalNonZeroPubkey,
    bytemuck::{Pod, Zeroable},
};

/// Close authority extension data for mints.
#[repr(C)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MintCloseAuthority {
    /// Optional authority to close the mint
    pub close_authority: OptionalNonZeroPubkey,
}
impl Extension for MintCloseAuthority {
    const TYPE: ExtensionType = ExtensionType::MintCloseAuthority;
}
