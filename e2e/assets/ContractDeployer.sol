// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ContractDeployer {
    function deploy(bytes memory bytecode) public returns (address contractAddress) {
        assembly {
            contractAddress := create(0, add(bytecode, 0x20), mload(bytecode))
            if iszero(contractAddress) {
                revert(0, 0)
            }
        }
    }
}
