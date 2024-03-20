// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

contract HelloWorld {
    constructor() payable {
    }
    function sayHelloWorld() public pure returns (string memory) {
        return "Hello World";
    }
    function getBalanceAsStr(address addr) public view  returns (string memory) {
        uint256 balance = addr.balance;
        return toString(balance);
    }
    function getSelfBalanceAsStr() public view  returns (string memory) {
        uint256 balance = address(this).balance;
        return toString(balance);
    }
    function toString(uint256 value) internal pure returns (string memory) {
        if (value == 0) {
            return "0";
        }

        uint256 temp = value;
        uint256 digits;

        while (temp != 0) {
            digits++;
            temp /= 10;
        }

        bytes memory buffer = new bytes(digits);

        while (value != 0) {
            digits--;
            buffer[digits] = bytes1(uint8(48 + (value % 10)));
            value /= 10;
        }

        return string(buffer);
    }
}