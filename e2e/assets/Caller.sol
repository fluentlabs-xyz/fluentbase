// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Caller {
    function callExternal(address target, bytes calldata data) external returns (bool success, bytes memory result) {
        (success, result) = target.call(data);
        return (success, result);
    }
}
