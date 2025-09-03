use serde::{Deserialize, Serialize};
use {
    crate::token_2022::extension::{Extension, ExtensionType},
    bytemuck::{Pod, Zeroable},
};

/// Indicates that the Account owner authority cannot be changed
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(transparent)]
pub struct ImmutableOwner;

impl Extension for ImmutableOwner {
    const TYPE: ExtensionType = ExtensionType::ImmutableOwner;
}
