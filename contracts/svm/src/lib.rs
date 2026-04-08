#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::entrypoint;
#[allow(unused_imports)]
use fluentbase_svm::fluentbase::loader_v4::{deploy_entry, main_entry};

entrypoint!(main_entry, deploy_entry);
