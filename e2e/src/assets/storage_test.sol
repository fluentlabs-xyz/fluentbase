// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

contract Storage {
    uint256 private value;
    constructor() payable {
        value = 100;
    }
    function getValue() public view returns (uint256) {
        return value;
    }
}