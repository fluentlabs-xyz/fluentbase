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

fn u256_from_slice_try(value: &[u8]) -> Option<U256> {
    U256::try_from_be_slice(value)
}

#[test]
fn test_deploy_factory_and_universal_token() {
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
    // The placeholder "__$f7bb912695101d7377bc19c7d693b1b376$__" contains the hash:
    // f7bb912695101d7377bc19c7d693b1b376 (40 hex chars).
    // solc prints it in the .bin as "73__$f7bb912695101d7377bc19c7d693b1b376$__63..." etc.
    // After filtering to only hex digits we see just the hash, so we replace that.
    // with the library address (also 40 hex chars)
    let mut factory_hex = hex_line.clone();
    let placeholder_hex =
        "5f5f2466376262393132363935313031643733373762633139633764363933623162333736245f5f";
    let hash_hex = "f7bb912695101d7377bc19c7d693b1b376";
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
        .name(TOKEN_NAME.to_string())
        .symbol(TOKEN_SYMBOL.to_string())
        .decimals(TOKEN_DECIMALS)
        .initial_supply(token_initial_supply)
        .minter(Address::ZERO)
        .pauser(Address::ZERO)
        .build()
        .create_deployment_transaction();

    println!(
        "Generated deployment data using Rust SDK (length: {} bytes)",
        rust_deploy_data.len()
    );
    assert_eq!(
        sdk_deploy_data.as_ref(),
        rust_deploy_data.as_ref(),
        "Solidity SDK deployment data must match Rust SDK data (sdk_len={}, rust_len={})",
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

    // Step 3: Compute the token address using computeTokenAddressString (CREATE2 formula)
    let chain_id = 1u64;
    let l1_token = address!("1111111111111111111111111111111111111111");
    sol! {
        function computeTokenAddressString(
            address l1Token,
            uint256 chainId,
            string memory name,
            string memory symbol,
            uint8 decimals,
            uint256 initialSupply,
            address minter,
            address pauser
        ) public view returns (address tokenAddress);
    }
    let compute_token_address_call = computeTokenAddressStringCall {
        l1Token: l1_token,
        chainId: U256::from(chain_id),
        name: TOKEN_NAME.to_string(),
        symbol: TOKEN_SYMBOL.to_string(),
        decimals: TOKEN_DECIMALS,
        initialSupply: token_initial_supply,
        minter: Address::ZERO,
        pauser: Address::ZERO,
    }
    .abi_encode();
    let compute_token_address_result = ctx.call_evm_tx(
        DEPLOYER,
        factory_address,
        compute_token_address_call.into(),
        Some(50_000_000),
        None,
    );
    assert!(
        compute_token_address_result.is_success(),
        "computeTokenAddressString failed: {:?}",
        compute_token_address_result
    );
    let compute_token_address_output = compute_token_address_result.output().unwrap();
    let computed_address: Address = SolidityABI::decode(compute_token_address_output, 0).unwrap();
    println!("Computed token address: {:?}", computed_address);

    // Step 4: Deploy Universal Token via factory using deployBridgedTokenCreate2 (CREATE2)
    sol! {
        function deployBridgedTokenCreate2(
            address l1Token,
            uint256 chainId,
            string memory name,
            string memory symbol,
            uint8 decimals,
            uint256 initialSupply,
            address minter,
            address pauser
        ) public returns (address tokenAddress);
    }

    let deploy_call = deployBridgedTokenCreate2Call {
        l1Token: l1_token,
        chainId: U256::from(chain_id),
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
        "deployBridgedTokenCreate2 failed: {:?}",
        deploy_result
    );

    let deploy_output = deploy_result.output().unwrap();
    let deployed_address: Address = SolidityABI::decode(deploy_output, 0).unwrap();
    println!("Deployed token address via factory: {:?}", deployed_address);

    assert_eq!(
        deployed_address, computed_address,
        "CREATE2 deployed address does not match computed address"
    );

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
        "Initial supply is wrong"
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
