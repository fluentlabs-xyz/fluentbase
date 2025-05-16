#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};
use libsecp256k1::{verify, Message, PublicKey, Signature};

pub fn main_entry(sdk: impl SharedAPI) {
    // check input size
    let input_size = sdk.input_size();
    assert_eq!(input_size, 32 + 64 + 65);

    // parse the message
    let mut message = [0u8; 32];
    sdk.read(&mut message, 0);
    let message = Message::parse(&message);

    // parse the signature - ensuring it's in the right format
    let mut signature = [0u8; 64];
    sdk.read(&mut signature, 32);
    let signature =
        Signature::parse_standard(&signature).unwrap_or_else(|_| panic!("can't parse signature"));

    // parse the public key
    let mut public_key = [0u8; 65];
    sdk.read(&mut public_key, 32 + 64);
    let public_key =
        PublicKey::parse(&public_key).unwrap_or_else(|_| panic!("can't parse public key"));

    // verify the signature
    let is_ok = verify(&message, &signature, &public_key);
    assert!(is_ok, "signature verification failed");
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;
    use hex_literal::hex;
    use libsecp256k1::{sign, Message, PublicKey, SecretKey};
    use tiny_keccak::{Hasher, Keccak};

    #[test]
    fn test_contract_works() {
        // Create private key and derive public key
        let secret_key = SecretKey::parse(&hex!(
            "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
        ))
        .unwrap();
        let public_key = PublicKey::from_secret_key(&secret_key);
        let public_key = public_key.serialize();

        // Create a message hash
        let mut keccak256 = Keccak::v256();
        let mut digest = [0u8; 32];
        keccak256.update("Hello, World".as_bytes());
        keccak256.finalize(&mut digest);

        // Create the message and sign it
        let message = Message::parse(&digest);
        let (signature, _recovery_id) = sign(&message, &secret_key);
        let signature = signature.serialize();

        // Combine the message, signature, and public key
        let mut input: Vec<u8> = vec![];
        input.extend(&digest);
        input.extend_from_slice(&signature);
        input.extend_from_slice(&public_key);

        println!("input: {:?}", hex::encode(&input));

        let sdk = HostTestingContext::default().with_input(input);
        main_entry(sdk.clone());
    }
}
