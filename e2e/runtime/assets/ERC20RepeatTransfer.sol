// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IERC20 {
    function transfer(address to, uint256 value) external returns (bool);
}

/// @title ERC20RepeatTransfer
/// @notice Calls `transfer(to, amount)` on any ERC20 token `times` times.
/// @dev The token address, recipient, amount, and repetition count are all passed as input.
contract ERC20RepeatTransfer {
    error ZeroTimes();
    error TransferFailed(uint256 iteration);

    function repeatTransfer(
        address token,
        address to,
        uint256 amount,
        uint256 times
    ) external {
        if (times == 0) revert ZeroTimes();

        for (uint256 i = 0; i < times; ++i) {
            bool ok = IERC20(token).transfer(to, amount);
            if (!ok) revert TransferFailed(i);
        }
    }
}