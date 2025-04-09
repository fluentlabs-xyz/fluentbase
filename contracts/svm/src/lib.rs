#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;
extern crate core;
use fluentbase_sdk::func_entrypoint;
#[allow(unused_imports)]
use solana_ee_core::fluentbase::loader_v4::{deploy, main};

func_entrypoint!(main, deploy);
