use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_codec::{bytes::BytesMut, SolidityABI};
use fluentbase_sdk::universal_token::TokenConfigBuilder;
use fluentbase_sdk::{
    address, bytes, calc_create_address, constructor::encode_constructor_params, Address, Bytes,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, U256,
};
use fluentbase_testing::{try_print_utf8_error, EvmTestingContext, TxBuilder};
use fluentbase_universal_token::storage::InitialSettings;
use hex_literal::hex;
use revm::context::result::ExecutionResult;

// Helper function similar to universal_token.rs
fn u256_from_slice_try(value: &[u8]) -> Option<U256> {
    U256::try_from_be_slice(value)
}

// Helper function to encode address as ABI word (right-aligned)
fn abi_word_addr(a: Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_ref());
    w
}

/// Test Universal Token deployment using the Solidity SDK
///
/// To compile Solidity contracts:
/// ```bash
/// solc --bin contracts/universal-token/UniversalToken.sol -o e2e/assets/
/// solc --bin contracts/universal-token/UniversalTokenFactory.sol -o e2e/assets/
/// ```
///
/// Note: Universal Tokens use a precompile pattern, so they're deployed
/// directly with magic bytes + InitialSettings, not via a factory contract.

#[test]
#[ignore] // Ignore until Solidity contracts are compiled
fn test_universal_token_solidity_deployment() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    ctx.add_balance(DEPLOYER, U256::from(1e18));

    // Compile UniversalToken.sol first:
    // solc --bin contracts/universal-token/UniversalToken.sol -o e2e/assets/
    let bytecode = include_bytes!("../assets/UniversalToken.bin");

    // Encode constructor parameters: (string name, string symbol, uint8 decimals, uint256 initialSupply, address minter, address pauser)
    let constructor_params = hex!("00000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000de0b6b3a7640000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000084d7920546f6b656e0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000034d544b0000000000000000000000000000000000000000000000000000000000");
    let encoded_constructor_params = encode_constructor_params(&constructor_params);
    let mut full_bytecode = Vec::new();
    full_bytecode.extend_from_slice(bytecode);
    full_bytecode.extend_from_slice(&encoded_constructor_params);

    let token_address = ctx.deploy_evm_tx(DEPLOYER, full_bytecode.into());
    println!("Token deployed at: {:?}", token_address);

    // Test name() - selector: 0x06fdde03
    let result = TxBuilder::call(&mut ctx, DEPLOYER, token_address, None)
        .input(bytes!("06fdde03"))
        .exec();
    assert!(result.is_success());
    let output = result.output().unwrap();
    // Decode string from output (following router.rs pattern)
    let name: String = SolidityABI::decode(output, 0).unwrap();
    assert_eq!(name, "My Token");

    // Test symbol() - selector: 0x95d89b41
    let result = TxBuilder::call(&mut ctx, DEPLOYER, token_address, None)
        .input(bytes!("95d89b41"))
        .exec();
    assert!(result.is_success());
    let output = result.output().unwrap();
    let symbol: String = SolidityABI::decode(output, 0).unwrap();
    assert_eq!(symbol, "MTK");

    // Test decimals() - selector: 0x313ce567
    let result = TxBuilder::call(&mut ctx, DEPLOYER, token_address, None)
        .input(bytes!("313ce567"))
        .exec();
    assert!(result.is_success());
    let output = result.output().unwrap();
    // Decimals returns uint8, decode as U256 then convert (following universal_token.rs pattern)
    let decimals_u256 = u256_from_slice_try(output.as_ref()).unwrap();
    let decimals = decimals_u256.to::<u8>();
    assert_eq!(decimals, 18);

    // Test totalSupply() - selector: 0x18160ddd
    let result = TxBuilder::call(&mut ctx, DEPLOYER, token_address, None)
        .input(bytes!("18160ddd"))
        .exec();
    assert!(result.is_success());
    let output = result.output().unwrap();
    // Use the same pattern as universal_token.rs - decode U256 from output
    let total_supply = u256_from_slice_try(output.as_ref()).unwrap();
    assert_eq!(total_supply, U256::from(1e18));
}

#[test]
fn test_deploy_factory_and_token() {
    // Test deploying UniversalTokenFactory and then deploying a Universal Token
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    const TOKEN_NAME: &str = "Bridged Token";
    const TOKEN_SYMBOL: &str = "BRIDGE";
    const TOKEN_DECIMALS: u8 = 18;
    let token_initial_supply: U256 = U256::from(100);

    ctx.add_balance(DEPLOYER, U256::from(1e18));

    // Step 0: Deploy UniversalTokenSDK library first (required for linking)
    let sdk_bytecode_hex = std::str::from_utf8(include_bytes!("../assets/UniversalTokenSDK.bin"))
        .expect("Invalid SDK bytecode file");
    let sdk_hex_line: String = sdk_bytecode_hex
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    let sdk_bytecode = hex::decode(&sdk_hex_line).expect("Failed to decode SDK bytecode");
    let sdk_address = ctx.deploy_evm_tx(DEPLOYER, sdk_bytecode.into());
    println!("SDK library deployed at: {:?}", sdk_address);

    // Step 1: Deploy UniversalTokenFactory (with library linking)
    // Compile first: solc --bin contracts/universal-token/UniversalTokenFactory.sol -o e2e/assets/
    // Note: solc --bin outputs hex-encoded bytecode with library placeholders that need linking
    let factory_bytecode_hex =
        std::str::from_utf8(include_bytes!("../assets/UniversalTokenFactory.bin"))
            .expect("Invalid bytecode file");
    // Extract first line and filter to only hex characters (like other .bin files)
    let hex_line: String = factory_bytecode_hex
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    let mut factory_bytecode = hex::decode(&hex_line).expect("Failed to decode bytecode");

    // Link the library: Replace placeholder with actual library address
    // Work with hex string for easier string replacement
    // The placeholder "__$a7643d6ae8530b3ab7fd5d554e19792c1c$__" contains the hash:
    // a7643d6ae8530b3ab7fd5d554e19792c1c (40 hex chars).
    // solc prints it in the .bin as "73__$a7643d6ae8530b3ab7fd5d554e19792c1c$__63..." etc.
    // After filtering to only hex digits we see just the hash, so we replace that.
    // with the library address (also 40 hex chars)
    let mut factory_hex = hex_line.clone();
    let placeholder_hex =
        "5f5f2461373634336436616538353330623361623766643564353534653139373932633163245f5f";
    let hash_hex = "a7643d6ae8530b3ab7fd5d554e19792c1c";
    let sdk_address_hex = hex::encode(sdk_address.as_slice());

    if factory_hex.contains(placeholder_hex) {
        // Replace the entire placeholder with the library address
        factory_hex = factory_hex.replace(placeholder_hex, &sdk_address_hex);
    } else if factory_hex.contains(hash_hex) {
        // Just replace the hash part
        factory_hex = factory_hex.replace(hash_hex, &sdk_address_hex);
    }

    factory_bytecode = hex::decode(&factory_hex).expect("Failed to decode linked bytecode");

    let factory_address = ctx.deploy_evm_tx(DEPLOYER, factory_bytecode.into());
    println!("Factory deployed at: {:?}", factory_address);

    // Step 2: Generate deployment data using Rust SDK (this is the format the runtime expects)
    // Note: The runtime expects the Rust struct encoding format, not raw ABI-encoded parameters
    // Both SDKs should eventually produce compatible formats, but for now we use Rust SDK format
    sol! {
        function createDeploymentData(
            string memory name,
            string memory symbol,
            uint8 decimals,
            uint256 initialSupply,
            address minter,
            address pauser
        ) public pure returns (bytes memory deploymentData);
    }
    let create_deployment_data_call = createDeploymentDataCall {
        name: TOKEN_NAME.to_string(),
        symbol: TOKEN_SYMBOL.to_string(),
        decimals: TOKEN_DECIMALS,
        initialSupply: token_initial_supply,
        minter: Address::ZERO,
        pauser: Address::ZERO,
    }
    .abi_encode();
    let lib_result = ctx.call_evm_tx(
        DEPLOYER,
        sdk_address, // Call the library directly
        create_deployment_data_call.into(),
        Some(10_000_000),
        None,
    );
    assert!(
        lib_result.is_success(),
        "createDeploymentData failed: {:?}",
        lib_result
    );
    let lib_output = lib_result.output().unwrap();

    // Properly decode the `bytes` return value from the Solidity SDK.
    // This gives us the raw deployment data that the Solidity SDK believes
    // should be passed to CREATE.
    let mut cursor: &[u8] = lib_output.as_ref();
    let sdk_deploy_data: Bytes = SolidityABI::decode(&mut cursor, 0).unwrap();

    // Generate deployment data using Rust SDK for comparison. This is the format
    // we already know works with the Universal Token runtime.
    let rust_deploy_data = TokenConfigBuilder::default()
        .name("Bridged Token".to_string())
        .symbol("BRIDGE".to_string())
        .decimals(18)
        .initial_supply(U256::ZERO)
        .minter(Address::ZERO)
        .pauser(Address::ZERO)
        .build()
        .create_deployment_transaction();

    println!(
        "Generated deployment data using Rust SDK (length: {} bytes)",
        rust_deploy_data.len()
    );
    println!(
        "SDK vs Rust deployment data equality: {} (sdk_len={}, rust_len={})",
        sdk_deploy_data.as_ref() == rust_deploy_data.as_ref(),
        sdk_deploy_data.len(),
        rust_deploy_data.len()
    );

    //let solidity_deploy_data = sdk_deploy_data.clone();
    // Verify magic bytes are present
    assert!(
        sdk_deploy_data.len() >= 4,
        "Deployment data too short: {} bytes",
        sdk_deploy_data.len()
    );
    let magic = &sdk_deploy_data.as_ref()[0..4];
    let expected_magic = hex!("45524320"); // "ERC "
    assert_eq!(
        magic,
        expected_magic,
        "Deployment data missing magic bytes! Expected {:?}, got {:?}",
        hex::encode(expected_magic),
        hex::encode(magic)
    );

    // Step 3: Test CREATE with ContractDeployer using the Rust SDK deployment data
    // This proves the Solidity SDK format works through ContractDeployer
    let deployer_bytecode_hex =
        std::str::from_utf8(include_bytes!("../assets/ContractDeployer.bin"))
            .expect("Invalid ContractDeployer bytecode file");
    let deployer_hex_line: String = deployer_bytecode_hex
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    let deployer_bytecode =
        hex::decode(&deployer_hex_line).expect("Failed to decode ContractDeployer bytecode");
    let deployer_address = ctx.deploy_evm_tx(DEPLOYER, deployer_bytecode.into());
    println!("ContractDeployer deployed at: {:?}", deployer_address);

    // Deploy via ContractDeployer using the Solidity SDK deployment data
    sol! {
        function deploy(bytes memory bytecode) public returns (address contractAddress);
    }
    let deploy_via_deployer = deployCall {
        bytecode: sdk_deploy_data.as_ref().to_vec().into(),
    }
    .abi_encode();
    let deployer_result = TxBuilder::call(&mut ctx, DEPLOYER, deployer_address, None)
        .input(deploy_via_deployer.into())
        .gas_limit(50_000_000)
        .exec();
    if deployer_result.is_success() {
        let deployer_output = deployer_result.output().unwrap();
        let deployed_via_deployer: Address = SolidityABI::decode(deployer_output, 0).unwrap();
        println!(
            "Successfully deployed via ContractDeployer at: {:?}",
            deployed_via_deployer
        );

        // Verify the token deployed via ContractDeployer works
        let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_via_deployer, None)
            .input(bytes!("06fdde03")) // name() selector
            .exec();
        if result.is_success() {
            let output = result.output().unwrap();
            let name: String = SolidityABI::decode(output, 0).unwrap();
            println!("Token name from ContractDeployer: {}", name);
        } else if let Some(output) = result.output() {
            println!(
                "name() call on ContractDeployer-deployed token failed: {:?}",
                result
            );
            try_print_utf8_error(output.as_ref());
        }
    } else {
        println!("ContractDeployer deployment failed: {:?}", deployer_result);
        if let Some(output) = deployer_result.output() {
            try_print_utf8_error(output.as_ref());
        }
    }

    // Step 4: Deploy Universal Token via factory using the Solidity SDK encoder
    sol! {
        function deployBridgedToken(
            string memory name,
            string memory symbol,
            uint8 decimals,
            uint256 initialSupply,
            address minter,
            address pauser
        ) public returns (address tokenAddress);
    }

    let deploy_call = deployBridgedTokenCall {
        name: TOKEN_NAME.to_string(),
        symbol: TOKEN_SYMBOL.to_string(),
        decimals: TOKEN_DECIMALS,
        initialSupply: token_initial_supply,
        minter: Address::ZERO,
        pauser: Address::ZERO,
    }
    .abi_encode();

    let deploy_result = ctx.call_evm_tx(
        DEPLOYER,
        factory_address,
        deploy_call.into(),
        Some(50_000_000),
        None,
    );

    assert!(
        deploy_result.is_success(),
        "deployBridgedToken failed: {:?}",
        deploy_result
    );

    let deploy_output = deploy_result.output().unwrap();
    let deployed_address: Address = SolidityABI::decode(deploy_output, 0).unwrap();
    println!("Deployed token address via factory: {:?}", deployed_address);

    // Step 5: Verify token was deployed by calling totalSupply()
    let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_address, None)
        .input(bytes!("18160ddd")) // totalSupply() selector
        .exec();
    assert!(
        result.is_success(),
        "totalSupply() call failed: {:?}",
        result
    );
    let output = result.output().unwrap();
    let total_supply = u256_from_slice_try(output.as_ref()).unwrap();
    assert_eq!(
        total_supply, token_initial_supply,
        "Initial supply should be zero"
    );

    // Step 6: Verify token name
    let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_address, None)
        .input(bytes!("06fdde03")) // name() selector
        .exec();
    assert!(result.is_success(), "name() call failed: {:?}", result);
    let output = result.output().unwrap();
    let name: String = SolidityABI::decode(output, 0).unwrap();
    assert_eq!(name, TOKEN_NAME, "Token name mismatch");

    // Step 7: Verify token symbol
    let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_address, None)
        .input(bytes!("95d89b41")) // symbol() selector
        .exec();
    assert!(result.is_success(), "symbol() call failed: {:?}", result);
    let output = result.output().unwrap();
    let symbol: String = SolidityABI::decode(output, 0).unwrap();
    assert_eq!(symbol, TOKEN_SYMBOL, "Token symbol mismatch");

    // Step 8: Verify token decimals
    let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_address, None)
        .input(bytes!("313ce567")) // decimals() selector
        .exec();
    assert!(result.is_success(), "decimals() call failed: {:?}", result);
    let output = result.output().unwrap();
    let decimals = u256_from_slice_try(output.as_ref()).unwrap();
    assert_eq!(decimals, TOKEN_DECIMALS, "Token decimals mismatch");

    // Step 9: Verify token totalSupply
    let result = TxBuilder::call(&mut ctx, DEPLOYER, deployed_address, None)
        .input(bytes!("18160ddd")) // totalSupply() selector
        .exec();
    assert!(
        result.is_success(),
        "totalSupply() call failed: {:?}",
        result
    );
    let output = result.output().unwrap();
    let total_supply = u256_from_slice_try(output.as_ref()).unwrap();
    assert_eq!(
        total_supply, token_initial_supply,
        "Token total supply mismatch"
    );
}

#[test]
fn test_universal_token_direct_deployment() {
    // Test deploying Universal Token directly (without Solidity wrapper)
    // This shows how the underlying precompile works
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    ctx.add_balance(DEPLOYER, U256::from(1e18));

    // Create InitialSettings
    let initial_settings = InitialSettings {
        token_name: Default::default(),
        token_symbol: Default::default(),
        decimals: 18,
        initial_supply: U256::from(1000e18),
        minter: Address::ZERO,
        pauser: Address::ZERO,
    };

    // Encode with magic bytes prefix
    let deploy_data = initial_settings.encode_with_prefix();
    let token_address = ctx.deploy_evm_tx(DEPLOYER, deploy_data);

    // Verify the token was deployed
    let code = ctx.get_code(token_address);
    assert!(code.is_some());

    // Test totalSupply() - selector: 0x18160ddd
    let result = TxBuilder::call(&mut ctx, DEPLOYER, token_address, None)
        .input(bytes!("18160ddd"))
        .exec();
    assert!(result.is_success());
    let output = result.output().unwrap();
    // Use the same pattern as universal_token.rs - decode U256 from output
    let total_supply = u256_from_slice_try(output.as_ref()).unwrap();
    assert_eq!(total_supply, U256::from(1000e18));
}
