use crate::deploy_internal;

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/panic.wasm"))
}

pub fn main() {
    panic!("it is panic time")
}
