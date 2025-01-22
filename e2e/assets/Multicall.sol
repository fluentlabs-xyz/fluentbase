// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Multicall {
    /**
     * @dev Executes a batch of function calls on this contract.
     * @param data An array of encoded function calls
     * @return results An array containing the results of each function call
     */
    function multicall(bytes[] calldata data) external returns (bytes[] memory results) {
        results = new bytes[](data.length);

        // Execute each call and store its result
        for (uint256 i = 0; i < data.length; i++) {
            (bool success, bytes memory result) = address(this).delegatecall(data[i]);
            require(success, "Multicall: call failed");
            results[i] = result;
        }

        return results;
    }
}
