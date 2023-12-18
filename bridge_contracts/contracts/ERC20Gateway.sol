// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import {IBridge} from "./IBridge.sol";
import {IERC20Gateway} from "./IERC20Gateway.sol";

import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";

contract ERC20Gateway {
    modifier onlyBridgeSender() {
        require(msg.sender == bridgeContract, "call only from bridge");
        _;
    }

    address public bridgeContract;
    address public gatewayAuthority;

    event ReceivedTokens(address source, address target, uint256 amount);

    constructor(address _bridgeContract) payable {
        bridgeContract = _bridgeContract;
    }

    function sendTokens(
        address _token,
        address _to,
        uint256 _amount
    ) external payable {
        IERC20Upgradeable(_token).transferFrom(
            msg.sender,
            address(this),
            _amount
        );
        bytes memory _message = abi.encodeCall(
            ERC20Gateway.receiveTokens,
            (_token, msg.sender, _to, _amount)
        );

        IBridge(bridgeContract).sendMessage{value: msg.value}(_to, _message);
    }

    function receiveTokens(
        address _token,
        address _from,
        address _to,
        uint256 _amount
    ) external payable onlyBridgeSender {
        require(msg.value == 0, "Message value have to equal zero");

        IERC20Upgradeable(_token).transfer(_to, _amount);
        emit ReceivedTokens(_from, _to, _amount);
    }
}
