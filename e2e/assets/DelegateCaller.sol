// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract DelegateCaller {
    function sayHelloMyself(address _target) public {
        bytes memory data = abi.encodeWithSignature("sayHelloFromDelegator()");

        (bool success, bytes memory returnData) = _target.delegatecall(data);

        require(success, string(returnData));
    }

    function sayHelloFromDelegator() public pure returns (string memory){
        return "Hello from delegator";
    }

    function executeDelegateCall(address _target) public {
        bytes memory data = abi.encodeWithSignature("sayHelloWorld()");

        (bool success, bytes memory returnData) = _target.delegatecall(data);

        require(success, string(returnData));
    }
}