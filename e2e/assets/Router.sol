// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./Multicall.sol";

contract Router is Multicall {
    /// @notice Simple echo function that returns the input message
    /// @param message The message to echo back
    /// @return The same message that was passed in
    function greeting(string memory message) public pure returns (string memory) {
        return message;
    }

    /// @notice Another echo function with a different name
    /// @param message The message to echo back
    /// @return The same message that was passed in
    function customGreeting(string memory message) public pure returns (string memory) {
        return message;
    }
}
