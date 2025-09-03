use serde::{Deserialize, Serialize};
use {
    crate::token_2022::extension::{Extension, ExtensionType},
    bytemuck::{Pod, Zeroable},
};

/// Indicates that the tokens from this mint can't be transferred
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(transparent)]
pub struct NonTransferable;

/// Indicates that the tokens from this account belong to a non-transferable
/// mint
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(transparent)]
pub struct NonTransferableAccount;

impl Extension for NonTransferable {
    const TYPE: ExtensionType = ExtensionType::NonTransferable;
}

impl Extension for NonTransferableAccount {
    const TYPE: ExtensionType = ExtensionType::NonTransferableAccount;
}
