// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";

import {IBridge} from "./IBridge.sol";

interface IERC20Gateway {
    event ReceivedTokens(address target, uint256 amount);

    function sendTokens(address _token, address _to, uint256 _amount) external payable;

    function receiveTokens(address _token, address _from, address _to, uint256 _amount) external payable;
}
