// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;

import {IBridge} from "./IBridge.sol";
import {IERC20Gateway} from "./IERC20Gateway.sol";
import {ERC20PeggedToken} from "./ERC20PeggedToken.sol";
import {ERC20TokenFactory} from "./ERC20TokenFactory.sol";
import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract ERC20Gateway is Ownable {
    modifier onlyBridgeSender() {
        require(msg.sender == bridgeContract, "call only from bridge");
        _;
    }

    mapping(address => address) private tokenMapping;

    address public bridgeContract;
    address public gatewayAuthority;
    address public tokenFactory;

    event ReceivedTokens(address source, address target, uint256 amount);
    event UpdateTokenMapping(address indexed _originToken, address indexed _oldPeggedToken, address indexed _newPeggedToken);

    constructor(address _bridgeContract, address _tokenFactory) Ownable(msg.sender) payable {
        bridgeContract = _bridgeContract;
        tokenFactory = _tokenFactory;
    }

    function sendTokens(
        address _token,
        address _to,
        uint256 _amount,
        bytes calldata _tokenMetadata
    ) external payable {
        bytes memory _message;

        if (tokenMapping[_token] == address(0)) {
            (address originGateway, address originAddress) = ERC20PeggedToken(_token).getOrigin();

            require(tokenMapping[originAddress] == _token);

            ERC20PeggedToken(_token).burn(msg.sender, _amount);

            _message = abi.encodeCall(
                ERC20Gateway.receiveNativeTokens,
                (_token, originAddress, msg.sender, _to, _amount)
            );
        } else {
            address _peggedToken = tokenMapping[_token];
            require(_peggedToken != address(0), "no corresponding l2 token");

            IERC20Upgradeable(_token).transferFrom(
                msg.sender,
                address(this),
                _amount
            );

            _message = abi.encodeCall(
                ERC20Gateway.receivePeggedTokens,
                (_token, _peggedToken, msg.sender, _to, _amount, _tokenMetadata)
            );
        }


        IBridge(bridgeContract).sendMessage{value: msg.value}(_to, _message);
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

        if (_peggedToken.code.length > 0) {
            // first deposit,  mapping
            tokenMapping[_peggedToken] = _originToken;

            _deployL2Token(_tokenMetadata, _originToken);
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
    ) external payable onlyBridgeSender {
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

    function _deployL2Token(bytes memory _tokenMetadata, address _originToken) internal {
        address _peggedToken = ERC20TokenFactory(tokenFactory).deployPeggedToken(address(this), _originToken);
        (string memory _symbol, string memory _name, uint8 _decimals) = abi.decode(
            _tokenMetadata,
            (string, string, uint8)
        );
        ERC20PeggedToken(_peggedToken).initialize(_name, _symbol, _decimals, address(this), _originToken);
    }
}
