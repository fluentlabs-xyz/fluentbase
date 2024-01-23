// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import {IBridge} from "./IBridge.sol";
import {IERC20Gateway} from "./IERC20Gateway.sol";
import {ERC20PeggedToken} from "./ERC20PeggedToken.sol";
import {ERC20TokenFactory} from "./ERC20TokenFactory.sol";
import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract ERC20Gateway is Ownable {
    struct TokenMetadata {
        string symbol;
        string name;
        uint8 decimals;
    }

    modifier onlyBridgeSender() {
        require(msg.sender == bridgeContract, "call only from bridge");
        _;
    }

    mapping(address => address) private tokenMapping;

    address public bridgeContract;
    address public gatewayAuthority;
    address public tokenFactory;
    address public otherSide;
    address public otherSideTokenImplementation;
    address public otherSideFactory;

    event ReceivedTokens(address source, address target, uint256 amount);
    event UpdateTokenMapping(
        address indexed _originToken,
        address indexed _oldPeggedToken,
        address indexed _newPeggedToken
    );

    constructor(address _bridgeContract, address _tokenFactory) payable Ownable(msg.sender) {
        bridgeContract = _bridgeContract;
        tokenFactory = _tokenFactory;
    }

    function setOtherSide(address _otherSide, address _otherSideTokenImplementation, address _otherSideFactory) external payable onlyOwner {
        otherSide = _otherSide;
        otherSideTokenImplementation = _otherSideTokenImplementation;
        otherSideFactory = _otherSideFactory;
    }

    function computePeggedTokenAddress(address _token) external view returns (address) {
        return ERC20TokenFactory(tokenFactory).computePeggedTokenAddress(address(this), _token);
    }

    function computeOtherSidePeggedTokenAddress(address _token) external view returns (address) {
        return ERC20TokenFactory(tokenFactory).computeOtherSidePeggedTokenAddress(otherSide, _token, otherSideTokenImplementation, otherSideFactory);
    }

    function sendTokens(
        address _token,
        address _to,
        uint256 _amount
    ) external payable {
        bytes memory _message;

        if (tokenMapping[_token] == address(0)) {
            IERC20Upgradeable(_token).transferFrom(msg.sender, address(this), _amount);

            bytes memory rawTokenMetadata = abi.encode(
                ERC20(_token).symbol(),
                ERC20(_token).name(),
                ERC20(_token).decimals()
            );

            address peggedToken = ERC20TokenFactory(tokenFactory).computeOtherSidePeggedTokenAddress(otherSide, _token, otherSideTokenImplementation, otherSideFactory);
            _message = abi.encodeCall(
                ERC20Gateway.receivePeggedTokens,
                (_token, peggedToken, msg.sender, _to, _amount, rawTokenMetadata)
            );
        } else {
            (address originGateway, address originAddress) = ERC20PeggedToken(_token).getOrigin();
            require(tokenMapping[_token] == originAddress);

            ERC20PeggedToken(_token).burn(msg.sender, _amount);

            _message = abi.encodeCall(
                ERC20Gateway.receiveNativeTokens,
                (_token, originAddress, msg.sender, _to, _amount)
            );
        }

        IBridge(bridgeContract).sendMessage{value: msg.value}(otherSide, _message);
    }

    function receivePeggedTokens(
        address _originToken,
        address _peggedToken,
        address _from,
        address _to,
        uint256 _amount,
        bytes calldata _tokenMetadata
    ) external payable onlyBridgeSender {
        require(msg.value == 0, "Message value have to equal zero");
        require(_originToken != address(0), "Origin token can't be equal zero");

        if (_peggedToken.code.length == 0) {
            address new_pegged_token = _deployL2Token(_tokenMetadata, _originToken);
            require(new_pegged_token == _peggedToken, "321Wrong pegged token provided as argument");

            tokenMapping[_peggedToken] = _originToken;
        } else {
            require(
                tokenMapping[_peggedToken] == _originToken,
                "123Failed while token mapping check. Origin or pegged token is wrong"
            );
        }

        ERC20PeggedToken(_peggedToken).mint(_to, _amount);
        emit ReceivedTokens(_from, _to, _amount);
    }

    function receiveNativeTokens(
        address _peggedToken,
        address _originToken,
        address _from,
        address _to,
        uint256 _amount
    ) external payable  {
        require(msg.value == 0, "Message value have to equal zero");

        IERC20Upgradeable(_originToken).transfer(_to, _amount);
        emit ReceivedTokens(_from, _to, _amount);
    }

    function updateTokenMapping(address _originToken, address _peggedToken) external onlyOwner {
        require(_peggedToken != address(0), "token address cannot be 0");

        address _oldPeggedToken = tokenMapping[_originToken];
        tokenMapping[_originToken] = _peggedToken;

        emit UpdateTokenMapping(_originToken, _oldPeggedToken, _peggedToken);
    }

    function _deployL2Token(bytes memory _tokenMetadata, address _originToken) internal returns (address) {
        address _peggedToken = ERC20TokenFactory(tokenFactory).deployPeggedToken(address(this), _originToken);
        (string memory _symbol, string memory _name, uint8 _decimals) = abi.decode(
            _tokenMetadata,
            (string, string, uint8)
        );
        ERC20PeggedToken(_peggedToken).initialize(_name, _symbol, _decimals, address(this), _originToken);

        return _peggedToken;
    }
}
