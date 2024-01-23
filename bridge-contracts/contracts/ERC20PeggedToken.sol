// SPDX-License-Identifier: GPL-3.0-only
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract ERC20PeggedToken is ERC20 {
    // we store symbol and name as bytes32
    string internal _symbol;
    string internal _name;

    // cross chain bridge (owner)
    address internal _owner;

    // origin address and chain id
    address internal _originAddress;
    address internal _gateway;

    uint8 private _decimals;

    constructor() ERC20("", "") {}

    function initialize(
        string memory name_,
        string memory symbol_,
        uint8 decimals_,
        address gateway,
        address originAddress
    ) public emptyOwner {
        _owner = msg.sender;
        _symbol = symbol_;
        _name = name_;
        _originAddress = originAddress;
        _gateway = gateway;
        _decimals = decimals_;
    }

    modifier emptyOwner() {
        require(_owner == address(0x00));
        _;
    }

    modifier onlyOwner() {
        require(msg.sender == _owner, "only owner");
        _;
    }

    function getOrigin() public view returns (address, address) {
        return (_gateway, _originAddress);
    }

    function mint(address account, uint256 amount) external onlyOwner {
        _mint(account, amount);
    }

    function burn(address account, uint256 amount) external onlyOwner {
        _burn(account, amount);
    }

    function name() public view override returns (string memory) {
        return _name;
    }

    function symbol() public view override returns (string memory) {
        return _symbol;
    }
}
