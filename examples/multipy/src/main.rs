#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, ExitCode, SharedAPI};


fn wrapping_div(a: u64, b: u64) -> u64 {
    if b == 0 {
        0 // or any fallback value you prefer
    } else {
        a / b
    }
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_length = sdk.input_size();
    assert!(input_length >= 16, "Expected at least 16 bytes");

    let mut input = alloc_slice(16);
    sdk.read(&mut input, 0);

    let a = u64::from_le_bytes(input[0..8].try_into().unwrap());
    let b = u64::from_le_bytes(input[8..16].try_into().unwrap());

    let result =  a / b;
    let result_bytes = result.to_le_bytes();

    sdk.write(&result_bytes);
}


entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{calc_create_address, Address, Bytes, ContractContextV1, FUEL_DENOM_RATE};
    use fluentbase_sdk_testing::{HostTestingContext, include_this_wasm, EvmTestingContext, TxBuilder, HostTestingContextNativeAPI};
    use rand::{Rng, SeedableRng};
    use rand::rngs::StdRng;

    const BYTECODE: &[u8] = include_this_wasm!();

    fn run_main(
        a: &[u8],
        b: &[u8],
    ) -> Vec<u8> {
        let mut ctx = EvmTestingContext::default();
        let deployer: Address = Address::ZERO;
        let mut builder = TxBuilder::create(&mut ctx, deployer, BYTECODE.into());
        let result = builder.exec();
        assert!(result.is_success(), "failed to deploy contract");
        let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&deployer, 0);
        let c: Vec<u8> = a.iter().copied().chain(b.iter().copied()).collect();
        let result = ctx.call_evm_tx(
            deployer,
            contract_address,
            Bytes::from(c),
            None,
            None,
        );
        assert!(result.is_success());
        result.output().unwrap().to_vec()
    }

    #[test]
    fn test_division() {
        let a: u64 = 15602080788219557311;
        let b: u64 = 9181438499313657906;
        let c = run_main(
            &a.to_le_bytes(),
            &b.to_le_bytes(),
        );
        let c = u64::from_le_bytes(c[0..8].try_into().unwrap());

        println!("WASM   {}/{}={}", a, b, c);
        println!("NATIVE {}/{}={}", a, b, a / b);
        if  a / b != c {
            panic!("Expected {} but got {}", a * b, c);
        }

    }

    #[test]
    fn test_hello_world_works() {
        let a: u64 = u64::MAX as u64;
        let b: u64 = u64::MAX as u64;
        let c = run_main(
            &a.to_le_bytes(),
            &b.to_le_bytes(),
        );
        let c = u64::from_le_bytes(c[0..8].try_into().unwrap());

        if  wrapping_div(a, a.wrapping_add(b).wrapping_mul(b)) != c {
            panic!("Expected {} but got {}", a * b, c);
        }
        println!("{}*{}={}", a, b, c);
    }

    #[test]
    fn test_multiplication_randomized() {
        let mut rng = StdRng::seed_from_u64(42); // deterministic randomness

        for _ in 0..100 {
            let a: u64 = rng.gen();
            let b: u64 = rng.gen();

            let result_bytes = run_main(&a.to_le_bytes(), &b.to_le_bytes());
            let result = u64::from_le_bytes(result_bytes[0..8].try_into().unwrap());

            let expected =  wrapping_div(a, a.wrapping_add(b).wrapping_mul(b));

            assert_eq!(
                result, expected,
                "Mismatch: {} * {} = {}, got {}",
                a, b, expected, result
            );
        }
    }

}
