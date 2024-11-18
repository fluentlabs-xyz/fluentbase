// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract CallWasm {
    constructor() payable {
    }
    function callWasm() public {
        address target = 0x1111111111111111111111111111111111111111;
        bytes memory data = "";
        bool success = false;
        assembly {
            let freeMemoryPointer := mload(0x40)

            let dataLength := mload(data)
            let dataPointer := add(data, 0x20)
            calldatacopy(freeMemoryPointer, dataPointer, dataLength)


            success := call(
                2000,
                target,
                0,
                freeMemoryPointer,
                dataLength,
                0,
                0
            )
        }
    }
}