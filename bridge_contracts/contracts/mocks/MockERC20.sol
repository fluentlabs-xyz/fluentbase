// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract MockERC20Token is ERC20 {
    constructor(
        string memory name,
        string memory symbol,
        uint256 initialSupply,
        address supplyTarget
    ) ERC20(name, symbol) {
        _mint(supplyTarget, initialSupply);
    }
}
