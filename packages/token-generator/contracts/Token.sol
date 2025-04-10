// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

contract Token is ERC20, Ownable {
    uint8 private immutable _decimals = 18;

    constructor(string memory _name, string memory _symbol, uint256 _initialSupply)
        ERC20(_name, _symbol)
        Ownable(msg.sender)  //  Fixed: Sets deployer as the owner
    {
        uint256 supply = _initialSupply * 10**_decimals;
        _mint(msg.sender, supply);  //  Deployer receives the total initial supply
    }

    function burn(uint256 amount) public {
        _burn(msg.sender, amount);
    }

    function decimals() public pure override returns (uint8) {
        return 18;
    }
}

