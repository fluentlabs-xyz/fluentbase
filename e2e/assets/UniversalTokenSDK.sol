// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title UniversalTokenSDK
 * @notice Solidity SDK for deploying and interacting with Universal Tokens
 * @dev Universal Tokens use a precompile/runtime pattern with magic bytes for deployment
 */
library UniversalTokenSDK {
    /// @notice Magic bytes prefix for Universal Token deployment (4 bytes: "ERC" + 0x20)
    bytes4 public constant UNIVERSAL_TOKEN_MAGIC_BYTES = bytes4(0x45524320); // "ERC "

    /// @notice Address of the Universal Token runtime precompile
    address public constant UNIVERSAL_TOKEN_RUNTIME = address(0x0000000000000000000000000000000000520008);

    /**
     * @notice Structure for Universal Token initial settings
     * @param tokenName Token name (max 32 bytes)
     * @param tokenSymbol Token symbol (max 32 bytes)
     * @param decimals Number of decimals (typically 18)
     * @param initialSupply Initial supply to mint to deployer
     * @param minter Optional minter address (zero address if not mintable)
     * @param pauser Optional pauser address (zero address if not pausable)
     */
    struct InitialSettings {
        bytes32 tokenName;
        bytes32 tokenSymbol;
        uint8 decimals;
        uint256 initialSupply;
        address minter;
        address pauser;
    }

    /**
     * @notice Creates deployment transaction data for a Universal Token
     * @param name Token name (will be truncated to 32 bytes)
     * @param symbol Token symbol (will be truncated to 32 bytes)
     * @param decimals Number of decimals
     * @param initialSupply Initial supply to mint
     * @param minter Minter address (address(0) if not mintable)
     * @param pauser Pauser address (address(0) if not pausable)
     * @return deploymentData Complete deployment data with magic bytes prefix
     * @dev Format matches Rust SDK exactly:
     *      UNIVERSAL_TOKEN_MAGIC_BYTES (4 bytes) +
     *      SolidityABI::encode(InitialSettings{TokenNameOrSymbol, TokenNameOrSymbol, u8, U256, Address, Address})
     *      where TokenNameOrSymbol is a transparent wrapper over [u8; 32].
     *
     *      Layout (after the 4-byte magic):
     *      - token_name: 32 * 32-byte words, one word per byte of the 32-byte name
     *      - token_symbol: 32 * 32-byte words
     *      - decimals: u8 stored in the last byte of a 32-byte word
     *      - initial_supply: uint256 as 32-byte big-endian
     *      - minter: address right-aligned in 32 bytes
     *      - pauser: address right-aligned in 32 bytes
     */
    function createDeploymentData(
        string memory name,
        string memory symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public pure returns (bytes memory deploymentData) {
        bytes32 nameBytes = stringToBytes32(name);
        bytes32 symbolBytes = stringToBytes32(symbol);
        deploymentData = _encodeInitialSettingsRustCompatible(
            nameBytes,
            symbolBytes,
            decimals,
            initialSupply,
            minter,
            pauser
        );
    }

    /**
     * @notice Computes a salt for bridge token deployment from L1 token address
     * @param l1Token L1 token address
     * @param chainId Chain ID to ensure uniqueness across chains
     * @return salt Deterministic salt for CREATE2
     */
    function computeBridgeTokenSalt(
        address l1Token,
        uint256 chainId
    ) internal pure returns (bytes32 salt) {
        return keccak256(abi.encodePacked("BRIDGE_TOKEN", l1Token, chainId));
    }

    // NOTE: Previously this SDK provided CREATE2-based address prediction helpers:
    // - computeTokenAddress
    // - computeBridgedTokenAddress
    //
    // In practice, Universal Tokens are deployed via CREATE with magic-bytes-prefixed
    // constructor data and a shared runtime (precompile pattern). Relying on CREATE2
    // for deployment was brittle in this environment and caused deployment failures.
    //
    // The factory now deploys tokens via CREATE and simply uses the returned address,
    // so we intentionally remove the CREATE2-based helpers from the public API.

    /**
     * @notice Converts bytes32 to string (truncates at first null byte)
     * @param data bytes32 value
     * @return str String representation
     */
    function bytes32ToString(bytes32 data) internal pure returns (string memory str) {
        // Convert bytes32 to bytes first, then find length
        bytes memory bytesArray = new bytes(32);
        assembly {
            mstore(add(bytesArray, 32), data)
        }

        // Find first null byte
        uint8 length = 0;
        while (length < 32 && bytesArray[length] != 0) {
            length++;
        }

        // Create new bytes array with correct length
        bytes memory result = new bytes(length);
        for (uint8 i = 0; i < length; i++) {
            result[i] = bytesArray[i];
        }
        return string(result);
    }

    /**
     * @notice Creates deployment transaction data for a Universal Token (bytes32 version)
     * @param name Token name as bytes32
     * @param symbol Token symbol as bytes32
     * @param decimals Number of decimals
     * @param initialSupply Initial supply to mint
     * @param minter Minter address (address(0) if not mintable)
     * @param pauser Pauser address (address(0) if not pausable)
     * @return deploymentData Complete deployment data with magic bytes prefix
     * @dev Matches Rust SDK encoding exactly:
     *      Rust: output.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
     *            output.extend_from_slice(encoded.as_ref());
     *      Where encoded is abi.encode(bytes32, bytes32, uint8, uint256, address, address)
     *      Format: magic_bytes (4) + encoded_struct_data
     *      Note: abi.encode returns bytes memory with length prefix, we extract just the data
     */
    function createDeploymentDataBytes32(
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) internal pure returns (bytes memory deploymentData) {
        deploymentData = _encodeInitialSettingsRustCompatible(
            name,
            symbol,
            decimals,
            initialSupply,
            minter,
            pauser
        );
    }

    /**
     * @notice Converts a string to bytes32 (truncates if longer than 32 bytes)
     * @param str Input string
     * @return result bytes32 representation
     */
    function stringToBytes32(string memory str) public pure returns (bytes32 result) {
        bytes memory tempBytes = bytes(str);
        if (tempBytes.length == 0) {
            return 0x0;
        }

        if (tempBytes.length <= 32) {
            assembly {
                result := mload(add(tempBytes, 32))
            }
        } else {
            // Truncate to 32 bytes
            assembly {
                result := mload(add(tempBytes, 32))
            }
        }
    }

    /**
     * @notice Internal helper that encodes InitialSettings using the same layout
     *         as Rust's SolidityABI::encode(InitialSettings), including magic prefix.
     */
    function _encodeInitialSettingsRustCompatible(
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) internal pure returns (bytes memory deploymentData) {
        // 4 bytes magic + 2 * 32 (bytes) * 32 (per-byte words) + 4 * 32 (decimals, supply, minter, pauser)
        uint256 TOTAL_LEN = 4 + 2 * 32 * 32 + 4 * 32; // 2180
        deploymentData = new bytes(TOTAL_LEN);

        uint256 offset = 0;

        // Magic bytes "ERC "
        deploymentData[0] = 0x45; // 'E'
        deploymentData[1] = 0x52; // 'R'
        deploymentData[2] = 0x43; // 'C'
        deploymentData[3] = 0x20; // ' '
        offset = 4;

        // Encode token_name: 32 bytes, each as a 32-byte word with the byte in the last position
        for (uint256 i = 0; i < 32; i++) {
            bytes1 b = name[i];
            uint256 wordStart = offset + i * 32;
            deploymentData[wordStart + 31] = b;
        }
        offset += 32 * 32; // 1024

        // Encode token_symbol: same pattern
        for (uint256 i = 0; i < 32; i++) {
            bytes1 b = symbol[i];
            uint256 wordStart = offset + i * 32;
            deploymentData[wordStart + 31] = b;
        }
        offset += 32 * 32; // +1024 => 2048 after magic

        // Encode decimals: u8 stored in the last byte of a 32-byte word
        deploymentData[offset + 31] = bytes1(decimals);
        offset += 32;

        // Encode initialSupply: uint256 as 32-byte big-endian word
        bytes32 supplyBE = bytes32(initialSupply);
        for (uint256 i = 0; i < 32; i++) {
            deploymentData[offset + i] = supplyBE[i];
        }
        offset += 32;

        // Encode minter: address right-aligned in 32 bytes
        bytes32 minterBE = bytes32(uint256(uint160(minter)));
        for (uint256 i = 0; i < 32; i++) {
            deploymentData[offset + i] = minterBE[i];
        }
        offset += 32;

        // Encode pauser: address right-aligned in 32 bytes
        bytes32 pauserBE = bytes32(uint256(uint160(pauser)));
        for (uint256 i = 0; i < 32; i++) {
            deploymentData[offset + i] = pauserBE[i];
        }
    }

    /**
     * @notice Deploys a Universal Token using CREATE
     * @param name Token name (will be truncated to 32 bytes if longer)
     * @param symbol Token symbol (will be truncated to 32 bytes if longer)
     * @param decimals Number of decimals (typically 18)
     * @param initialSupply Initial supply to mint to deployer
     * @param minter Minter address (address(0) if not mintable)
     * @param pauser Pauser address (address(0) if not pausable)
     * @return tokenAddress Address of the deployed token
     * @dev Uses CREATE opcode; the actual address is returned by the EVM.
     *      This is the recommended way to deploy Universal Tokens directly.
     */
    function deployToken(
        string memory name,
        string memory symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public returns (address tokenAddress) {
        bytes memory deploymentData = createDeploymentData(
            name,
            symbol,
            decimals,
            initialSupply,
            minter,
            pauser
        );

        assembly {
            // CREATE(value, offset, length)
            // value = 0 (no ETH sent)
            // offset = deploymentData + 0x20 (skip length word)
            // length = mload(deploymentData) (get length from first word)
            let dataPtr := add(deploymentData, 0x20)
            let dataLen := mload(deploymentData)
            tokenAddress := create(0, dataPtr, dataLen)
        }

        require(tokenAddress != address(0), "UniversalTokenSDK: deployment failed");
    }

    /**
     * @notice EXPERIMENTAL: Try encoding as a struct-like format
     * @dev This attempts to match Rust's struct encoding which might include metadata
     *      The Rust struct has nested TokenNameOrSymbol structs which might encode differently
     */
    function createDeploymentDataExperimental(
        string memory name,
        string memory symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public pure returns (bytes memory deploymentData) {
        bytes32 nameBytes = stringToBytes32(name);
        bytes32 symbolBytes = stringToBytes32(symbol);

        // Try encoding as if it's a struct with nested structs
        // Rust's InitialSettings struct has:
        //   token_name: TokenNameOrSymbol (which is #[repr(transparent)] [u8; 32])
        //   token_symbol: TokenNameOrSymbol
        //   decimals: u8
        //   initial_supply: U256
        //   minter: Address
        //   pauser: Address

        // When Rust encodes a struct with Codec derive, it might add:
        // - Struct metadata/offsets for nested structs
        // - Length prefixes for dynamic types
        // Let's try encoding each field as if TokenNameOrSymbol is a struct itself

        // For now, just use the simple format and see what the actual difference is
        // We'll need to reverse-engineer the Rust encoding format
        bytes memory encoded = abi.encode(
            nameBytes,
            symbolBytes,
            decimals,
            initialSupply,
            minter,
            pauser
        );

        // Prepend magic bytes
        uint256 dataLen = encoded.length;
        deploymentData = new bytes(4 + dataLen);

        assembly {
            let dataPtr := add(deploymentData, 32)
            mstore8(dataPtr, 0x45)
            mstore8(add(dataPtr, 1), 0x52)
            mstore8(add(dataPtr, 2), 0x43)
            mstore8(add(dataPtr, 3), 0x20)

            let encodedDataPtr := add(encoded, 32)
            let targetPtr := add(dataPtr, 4)

            // Copy all encoded data
            let copyLen := dataLen
            for { let i := 0 } lt(i, copyLen) { i := add(i, 32) } {
                mstore(add(targetPtr, i), mload(add(encodedDataPtr, i)))
            }
        }
    }

    /**
     * @notice EXPERIMENTAL: Debug function to see what Rust encoding might look like
     * @dev This helps us understand the format difference
     */
    function debugEncodingComparison(
        bytes32 name,
        bytes32 symbol,
        uint8 decimals,
        uint256 initialSupply,
        address minter,
        address pauser
    ) public pure returns (
        bytes memory simpleEncode,
        uint256 simpleLen,
        bytes memory structLikeEncode,
        uint256 structLikeLen
    ) {
        // Simple encoding (what we currently do)
        simpleEncode = abi.encode(name, symbol, decimals, initialSupply, minter, pauser);
        simpleLen = simpleEncode.length;

        // Try encoding as if TokenNameOrSymbol is a struct (might add metadata)
        // In Rust, TokenNameOrSymbol is #[repr(transparent)] with [u8; 32]
        // When encoded as part of InitialSettings struct, it might add struct metadata
        // For now, return the same (we need to reverse-engineer the actual format)
        structLikeEncode = simpleEncode;
        structLikeLen = structLikeEncode.length;
    }
}
