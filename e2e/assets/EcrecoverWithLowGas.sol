// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract EcrecoverWithLowGas {
    function callEcrecover() external {
        bytes32 hash = 0x5c7783d9be1f3d1a6e33ae0215f7ec17e0a1e12f1e935b982c3705fc3e45c3b0;
        uint8 v = 27;
        bytes32 r = 0xe1f0c2e02f979b14b3289d4ab758a409b15b3a2efedfb31c4d207d416ea5f2ca;
        bytes32 s = 0x3f7c1f7a9d70b9c2b17e5fd45f51cf2c142e2aa3a9b2dcd273028f0d7ecf20b2;

        bytes memory input = abi.encodePacked(hash, v, r, s);
        bytes memory result = new bytes(32);
        bool success;

        assembly {
            success := call(
                2300,            // gas
                0x01,            // ecrecover precompile address
                1,               // value
                add(input, 0x20),
                mload(input),
                add(result, 0x20),
                32
            )
        }

        require(success, "ecrecover failed");
    }
}
