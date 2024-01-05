// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

interface IBridge {
    event SentMessage(address indexed sender, address indexed to, uint256 value, bytes32 messageHash);

    event ReceivedMessage(bytes32 messageHash, bool successfulCall);

    function sendMessage(address _to, bytes calldata _message) external payable;

    function receiveMessage(
        address _from,
        address payable _to,
        uint256 _value,
        uint256 _nonce,
        bytes calldata _message
    ) external payable;
}
