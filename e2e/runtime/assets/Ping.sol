// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract Ping {
    event Ping(address caller, uint256 x);

    uint256 public value;

    constructor() {
        value = 1;
    }

    function ping(uint256 x) external returns (uint256) {
        value = x;
        emit Ping(msg.sender, x);
        return x + 1;
    }
}
