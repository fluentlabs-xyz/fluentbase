#![cfg_attr(target_arch = "wasm32", no_std)]

mod evm;
#[cfg(any(
    feature = "blake2",
    feature = "sha256",
    feature = "ripemd160",
    feature = "identity",
    feature = "modexp",
    feature = "ecrecover",
))]
mod precompile;
#[cfg(feature = "evm")]
mod svm;
mod wasm;

#[cfg(feature = "blake2")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::BlakeInvokeFunc>);
#[cfg(feature = "sha256")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::Sha256InvokeFunc>);
#[cfg(feature = "ripemd160")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::Ripemd160InvokeFunc>);
#[cfg(feature = "identity")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::IdentityInvokeFunc>);
#[cfg(feature = "modexp")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::ModexpInvokeFunc>);
#[cfg(feature = "ecrecover")]
fluentbase_sdk::basic_entrypoint!(precompile::PRECOMPILE<precompile::EcrecoverInvokeFunc>);

#[cfg(feature = "evm")]
fluentbase_sdk::basic_entrypoint!(
    evm::EVM<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
