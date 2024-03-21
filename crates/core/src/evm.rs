pub mod address;
pub mod balance;
pub mod call;
pub mod calldatacopy;
pub mod calldataload;
pub mod calldatasize;
pub mod codecopy;
pub mod codehash;
pub mod codesize;
// #[cfg(not(any(feature = "evm_loader", feature = "wcl")))]
pub mod create;
// #[cfg(not(any(feature = "evm_loader", feature = "wcl")))]
pub mod create2;
pub mod extcodecopy;
pub mod extcodehash;
pub mod extcodesize;
pub mod log0;
pub mod log1;
pub mod log2;
pub mod log3;
pub mod log4;
pub mod r#return;
pub mod revert;
pub mod selfbalance;
pub mod sload;
pub mod sstore;
