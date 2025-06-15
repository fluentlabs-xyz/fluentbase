// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Blake2FCaller {
    /**
     * @dev Calls the BLAKE2F precompile (0x09) with valid 213-byte input.
     * @return output 64-byte final state vector from the BLAKE2F function
     */
    function callBlake2F() external view returns (bytes memory output) {
        bytes memory input = new bytes(213);

        // Fill input with valid structure:
        // - 4 bytes: rounds (big-endian)
        // - 64 bytes: H (zeroed)
        // - 128 bytes: M (zeroed)
        // - 16 bytes: T (zeroed)
        // - 1 byte: final block = 1

        // Rounds = 1 (big-endian)
        input[0] = 0x00;
        input[1] = 0x00;
        input[2] = 0x00;
        input[3] = 0x01;

        // Final block = 1
        input[212] = 0x01;

        // Prepare output buffer
        output = new bytes(64);

        // Call precompile
        bool success;
        assembly {
            let inPtr := add(input, 32)
            let outPtr := add(output, 32)
            success := staticcall(gas(), 0x09, inPtr, 213, outPtr, 64)
        }

        require(success, "BLAKE2F call failed");
    }
}
