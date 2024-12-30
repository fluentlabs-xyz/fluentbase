// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract DelegateCaller {
    function sayHelloMyself(address _target) public returns (string memory)  {
        bytes memory data = abi.encodeWithSignature("sayHelloFromDelegator()");

        (bool success, bytes memory returnData) = _target.delegatecall(data);

        require(success, string(returnData));

        return string(returnData);
    }

    function sayHelloFromDelegator() public pure returns (string memory){
        return "Hello from delegator";
    }

    function executeDelegateCall(address _target) public returns (string memory) {
        bytes memory data = abi.encodeWithSignature("sayHelloWorld()");

        (bool success, bytes memory returnData) = _target.delegatecall(data);

        require(success, string(returnData));

        return string(returnData);
    }
}