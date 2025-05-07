#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI};

pub fn main(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    sdk.read(input, 0);
    let hash = sdk.keccak256(input); // calculate the hash via syscall to builtin keccak256
    sdk.write(&hash.as_slice());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{hex};
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = HostTestingContext::default().with_input("Hello, World");
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(
            &output[0..32],
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
    }
}
