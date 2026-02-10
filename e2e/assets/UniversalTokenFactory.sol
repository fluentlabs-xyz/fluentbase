// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./UniversalTokenSDK.sol";

/**
 * @title UniversalTokenFactory
 * @notice Factory contract for deploying Universal Tokens
 * @dev Provides deterministic token deployment for bridge integration
 */
contract UniversalTokenFactory {
    using UniversalTokenSDK for *;

    /// @notice Mapping from L1 token address to L2 Universal Token address
    mapping(address => address) public bridgedTokens;

    /// @notice Mapping from token address to deployment info
    mapping(address => TokenInfo) public tokenInfo;

    /// @notice Token deployment information
    struct TokenInfo {
        address l1Token;
        uint256 chainId;
        bool deployed;
    }

    /// @notice Emitted when a new Universal Token is deployed
    event TokenDeployed(
        address indexed l1Token,
        address indexed l2Token,
        string name,
        string symbol,
        uint8 decimals
    );

    /**
     * @notice Computes the address of a Universal Token for a given L1 token
     * @dev Uses CREATE2 semantics with the same salt and init code as
     *      `deployBridgedTokenCreate2`. This allows deterministic address
     *      prediction from inputs alone.
     */
    function computeTokenAddress(
        address l1Token,
        uint256 chainId,
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public view returns (address tokenAddress) {
        bytes memory deploymentData = UniversalTokenSDK.createDeploymentDataBytes32(
            name,
            symbol,
            decimals,
            initialSupply,
            minter,
            pauser
        );

        bytes32 salt = UniversalTokenSDK.computeBridgeTokenSalt(l1Token, chainId);
        bytes32 initCodeHash = keccak256(deploymentData);

        // Standard CREATE2 address formula:
        // address = keccak256(0xff ++ deployingAddr ++ salt ++ keccak256(init_code))[12:]
        bytes32 hash = keccak256(
            abi.encodePacked(bytes1(0xff), address(this), salt, initCodeHash)
        );
        tokenAddress = address(uint160(uint256(hash)));
    }

    /**
     * @notice Computes the address of a Universal Token for a given L1 token (string version)
     * @param l1Token L1 token address
     * @param chainId Chain ID of the L1 chain
     * @param name Token name
     * @param symbol Token symbol
     * @param decimals Number of decimals
     * @param initialSupply Initial supply
     * @param minter Minter address
     * @param pauser Pauser address
     * @return tokenAddress Predicted Universal Token address
     */
    function computeTokenAddressString(
        address l1Token,
        uint256 chainId,
        string memory name,
        string memory symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public view returns (address tokenAddress) {
        return computeTokenAddress(
            l1Token,
            chainId,
            UniversalTokenSDK.stringToBytes32(name),
            UniversalTokenSDK.stringToBytes32(symbol),
            decimals,
            initialSupply,
            minter,
            pauser
        );
    }

    /**
     * @notice Debug function to get the deployment data and bytecode hash
     * @dev This helps verify the encoding matches between Solidity and Rust
     */
    function getDeploymentDataAndHash(
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public pure returns (bytes memory deploymentData, bytes32 bytecodeHash) {
        deploymentData = UniversalTokenSDK.createDeploymentDataBytes32(
            name,
            symbol,
            decimals,
            initialSupply,
            minter,
            pauser
        );
        bytecodeHash = keccak256(deploymentData);
    }

    /**
     * @notice Debug function to check what abi.encode produces
     */
    function debugAbiEncode(
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public pure returns (bytes memory encoded, uint256 encodedLength) {
        encoded = abi.encode(name, symbol, decimals, initialSupply, minter, pauser);
        encodedLength = encoded.length;
    }

    /**
     * @notice EXPERIMENTAL: Try deploying with raw deployment data to test format
     * @dev This allows us to test if the deployment data format is correct
     */
    function deployBridgedTokenRaw(
        address l1Token,
        uint256 chainId,
        bytes memory deploymentData
    ) public returns (address tokenAddress) {
        require(l1Token != address(0), "UniversalTokenFactory: invalid L1 token");
        require(chainId > 0, "UniversalTokenFactory: invalid chain ID");
        require(
            bridgedTokens[l1Token] == address(0),
            "UniversalTokenFactory: token already deployed"
        );

        // Try deploying with the raw deployment data
        assembly {
            tokenAddress := create(0, add(deploymentData, 0x20), mload(deploymentData))
            if iszero(tokenAddress) {
                revert(0, 0)
            }
        }

        // Record deployment
       // bridgedTokens[l1Token] = tokenAddress;
        tokenInfo[tokenAddress] = TokenInfo({
            l1Token: l1Token,
            chainId: chainId,
            deployed: true
        });

        emit TokenDeployed(l1Token, tokenAddress, "", "", 0);
    }

    /**
     * @notice Deploys a Universal Token for a bridged L1 token using CREATE2
     * @param l1Token Original L1 token address (cannot be zero)
     * @param chainId Chain ID of the L1 chain (must be > 0)
     * @param name Token name (will be truncated to 32 bytes if longer)
     * @param symbol Token symbol (will be truncated to 32 bytes if longer)
     * @param decimals Number of decimals (typically 18)
     * @param initialSupply Initial supply to mint
     * @param minter Minter address (address(0) if not mintable)
     * @param pauser Pauser address (address(0) if not pausable)
     * @return tokenAddress Address of the deployed Universal Token
     * @dev Uses CREATE2 for deterministic deployment; address is
     *      keccak256(0xff ++ factory ++ salt ++ keccak256(init_code))[12:].
     */
    function deployBridgedTokenCreate2(
        address l1Token,
        uint256 chainId,
        string memory name,
        string memory symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public returns (address tokenAddress) {
        require(l1Token != address(0), "UniversalTokenFactory: invalid L1 token");
        require(chainId > 0, "UniversalTokenFactory: invalid chain ID");
        require(
            bridgedTokens[l1Token] == address(0),
            "UniversalTokenFactory: token already deployed"
        );

        bytes memory deploymentData = UniversalTokenSDK.createDeploymentData(
            name,
            symbol,
            decimals,
            initialSupply,
            minter,
            pauser
        );
        bytes32 salt = UniversalTokenSDK.computeBridgeTokenSalt(l1Token, chainId);

        assembly {
            tokenAddress := create2(0, add(deploymentData, 0x20), mload(deploymentData), salt)
            if iszero(tokenAddress) {
                revert(0, 0)
            }
        }

        bridgedTokens[l1Token] = tokenAddress;
        tokenInfo[tokenAddress] = TokenInfo({
            l1Token: l1Token,
            chainId: chainId,
            deployed: true
        });

        emit TokenDeployed(l1Token, tokenAddress, name, symbol, decimals);
    }
}
