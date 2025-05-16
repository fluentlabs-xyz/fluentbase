#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};
// #[allow(unused_imports)]
use fluentbase_svm::fluentbase::loader_v4::{deploy_entry, main_entry};

entrypoint!(main_entry2, deploy_entry2);

pub fn deploy_entry2<SDK: SharedAPI>(mut sdk: SDK) {
    deploy_entry(sdk);
}

pub fn main_entry2<SDK: SharedAPI>(mut sdk: SDK) {
    main_entry(sdk);
}
