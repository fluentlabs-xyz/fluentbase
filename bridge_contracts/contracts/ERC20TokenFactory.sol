// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract ERC20TokenFactory is Ownable {
    address public implementation;

    event TokenDeployed(address indexed _originToken, address indexed _peggedToken);

    constructor(address _implementation) Ownable(msg.sender) {
        require(_implementation != address(0), "zero implementation address");

        implementation = _implementation;
    }

    function computePeggedTokenAddress(address _gateway, address _originToken) external view returns (address) {
        bytes32 _salt = _calculateSalt(_gateway, _originToken);

        return Clones.predictDeterministicAddress(implementation, _salt);
    }

    function deployPeggedToken(address _gateway, address _originToken) external onlyOwner returns (address) {
        bytes32 salt = _calculateSalt(_gateway, _originToken);

        address peggedToken = Clones.cloneDeterministic(implementation, salt);

        emit TokenDeployed(_originToken, peggedToken);

        return peggedToken;
    }

    function _calculateSalt(address _gateway, address _originToken) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(_gateway, _originToken));
    }
}
