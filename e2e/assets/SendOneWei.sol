// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract SendOneWei {
    function sendOneWei(address payable target) external {
        (bool success, ) = target.call{value: 1}("");
        require(success, "Transfer failed");
    }
}