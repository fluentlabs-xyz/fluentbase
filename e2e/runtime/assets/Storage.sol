// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract Storage {
    event Test(uint256);
    uint256 private value;
    mapping(address => uint256) private balances;
    mapping(address => mapping(address => uint256)) private allowances;
    constructor() payable {
        value = 100;
        balances[msg.sender] = 100;
        allowances[msg.sender][address(this)] = 100;
    }
    function setValue(uint256 newValue) public {
        value = newValue;
        balances[msg.sender] = newValue;
        allowances[msg.sender][address(this)] = newValue;
        emit Test(value);
    }
    function getValue() public view returns (uint256) {
        require(balances[msg.sender] == value, "value mismatch");
        require(allowances[msg.sender][address(this)] == value, "value mismatch");
        return value;
    }
}