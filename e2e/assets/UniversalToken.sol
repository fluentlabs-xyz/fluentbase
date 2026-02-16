// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title IUniversalToken
 * @notice Interface for Universal Tokens (ERC20-compatible)
 * @dev Universal Tokens use a precompile/runtime pattern
 */
interface IUniversalToken {
    /**
     * @notice Returns the name of the token
     * @return name Token name
     */
    function name() external view returns (string memory name);

    /**
     * @notice Returns the symbol of the token
     * @return symbol Token symbol
     */
    function symbol() external view returns (string memory symbol);

    /**
     * @notice Returns the decimals of the token
     * @return decimals Number of decimals
     */
    function decimals() external view returns (uint8 decimals);

    /**
     * @notice Returns the total supply of the token
     * @return totalSupply Total token supply
     */
    function totalSupply() external view returns (uint256 totalSupply);

    /**
     * @notice Returns the balance of an account
     * @param account Address to query
     * @return balance Token balance
     */
    function balanceOf(address account) external view returns (uint256 balance);

    /**
     * @notice Transfers tokens to a recipient
     * @param to Recipient address
     * @param amount Amount to transfer
     * @return success True if transfer succeeded
     */
    function transfer(address to, uint256 amount) external returns (bool success);

    /**
     * @notice Transfers tokens from one address to another (requires approval)
     * @param from Source address
     * @param to Recipient address
     * @param amount Amount to transfer
     * @return success True if transfer succeeded
     */
    function transferFrom(
        address from,
        address to,
        uint256 amount
    ) external returns (bool success);

    /**
     * @notice Approves a spender to transfer tokens on behalf of the caller
     * @param spender Address to approve
     * @param amount Amount to approve
     * @return success True if approval succeeded
     */
    function approve(address spender, uint256 amount) external returns (bool success);

    /**
     * @notice Returns the allowance of a spender for an owner
     * @param owner Token owner
     * @param spender Approved spender
     * @return allowance Approved amount
     */
    function allowance(address owner, address spender) external view returns (uint256 allowance);

    /**
     * @notice Mints tokens to an address (only if minter is set)
     * @param to Recipient address
     * @param amount Amount to mint
     * @return success True if mint succeeded
     */
    function mint(address to, uint256 amount) external returns (bool success);

    /**
     * @notice Pauses token transfers (only if pauser is set)
     * @return success True if pause succeeded
     */
    function pause() external returns (bool success);

    /**
     * @notice Unpauses token transfers (only if pauser is set)
     * @return success True if unpause succeeded
     */
    function unpause() external returns (bool success);
}




/**
 * @title UniversalToken
 * @notice Solidity implementation of Universal Token Standard (ERC20-compatible)
 * @dev This is a standard ERC20 implementation with optional minting and pausing features
 *      Can be used as a reference implementation or deployed on non-Fluent chains
 */
contract UniversalToken is IUniversalToken {
    /// @notice Token name
    string private _name;

    /// @notice Token symbol
    string private _symbol;

    /// @notice Token decimals
    uint8 private _decimals;

    /// @notice Total token supply
    uint256 private _totalSupply;

    /// @notice Mapping from address to balance
    mapping(address => uint256) private _balances;

    /// @notice Mapping from owner to spender to allowance
    mapping(address => mapping(address => uint256)) private _allowances;

    /// @notice Optional minter address (if set, enables minting)
    address private _minter;

    /// @notice Optional pauser address (if set, enables pause/unpause)
    address private _pauser;

    /// @notice Paused state (true if transfers are paused)
    bool private _paused;

    /// @notice Emitted when tokens are transferred
    event Transfer(address indexed from, address indexed to, uint256 value);

    /// @notice Emitted when allowance is set
    event Approval(address indexed owner, address indexed spender, uint256 value);

    /// @notice Emitted when contract is paused
    event Paused(address indexed account);

    /// @notice Emitted when contract is unpaused
    event Unpaused(address indexed account);

    /// @notice Error thrown when operation is attempted while paused
    error EnforcedPause();

    /// @notice Error thrown when pause is expected but contract is not paused
    error ExpectedPause();

    /// @notice Error thrown when sender is invalid (zero address)
    error InvalidSender(address sender);

    /// @notice Error thrown when receiver is invalid (zero address)
    error InvalidReceiver(address receiver);

    /// @notice Error thrown when balance is insufficient
    error InsufficientBalance(address account, uint256 balance, uint256 required);

    /// @notice Error thrown when allowance is insufficient
    error InsufficientAllowance(address owner, address spender, uint256 allowance, uint256 required);

    /// @notice Error thrown when minting is not enabled
    error NotMintable();

    /// @notice Error thrown when caller is not the minter
    error MinterMismatch(address caller, address minter);

    /// @notice Error thrown when pausing is not enabled
    error NotPausable();

    /// @notice Error thrown when caller is not the pauser
    error PauserMismatch(address caller, address pauser);

    /**
     * @notice Constructor - initializes the token
     * @param name_ Token name
     * @param symbol_ Token symbol
     * @param decimals_ Number of decimals
     * @param initialSupply_ Initial supply to mint to deployer
     * @param minter_ Optional minter address (address(0) if not mintable)
     * @param pauser_ Optional pauser address (address(0) if not pausable)
     */
    constructor(
        string memory name_,
        string memory symbol_,
        uint8 decimals_,
        uint256 initialSupply_,
        address minter_,
        address pauser_
    ) {
        _name = name_;
        _symbol = symbol_;
        _decimals = decimals_;
        _minter = minter_;
        _pauser = pauser_;
        _paused = false;

        if (initialSupply_ > 0) {
            _balances[msg.sender] = initialSupply_;
            _totalSupply = initialSupply_;
            emit Transfer(address(0), msg.sender, initialSupply_);
        }
    }

    /**
     * @notice Returns the name of the token
     * @return Token name
     */
    function name() external view override returns (string memory) {
        return _name;
    }

    /**
     * @notice Returns the symbol of the token
     * @return Token symbol
     */
    function symbol() external view override returns (string memory) {
        return _symbol;
    }

    /**
     * @notice Returns the decimals of the token
     * @return Number of decimals
     */
    function decimals() external view override returns (uint8) {
        return _decimals;
    }

    /**
     * @notice Returns the total supply of the token
     * @return Total token supply
     */
    function totalSupply() external view override returns (uint256) {
        return _totalSupply;
    }

    /**
     * @notice Returns the balance of an account
     * @param account Address to query
     * @return Token balance
     */
    function balanceOf(address account) external view override returns (uint256) {
        return _balances[account];
    }

    /**
     * @notice Transfers tokens to a recipient
     * @param to Recipient address
     * @param amount Amount to transfer
     * @return success True if transfer succeeded
     */
    function transfer(address to, uint256 amount) external override returns (bool success) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    /**
     * @notice Transfers tokens from one address to another (requires approval)
     * @param from Source address
     * @param to Recipient address
     * @param amount Amount to transfer
     * @return success True if transfer succeeded
     */
    function transferFrom(
        address from,
        address to,
        uint256 amount
    ) external override returns (bool success) {
        address spender = msg.sender;

        // Check and update allowance
        uint256 currentAllowance = _allowances[from][spender];
        if (currentAllowance != type(uint256).max) {
            if (currentAllowance < amount) {
                revert InsufficientAllowance(from, spender, currentAllowance, amount);
            }
            unchecked {
                _allowances[from][spender] = currentAllowance - amount;
            }
        }

        _transfer(from, to, amount);
        return true;
    }

    /**
     * @notice Approves a spender to transfer tokens on behalf of the caller
     * @param spender Address to approve
     * @param amount Amount to approve
     * @return success True if approval succeeded
     */
    function approve(address spender, uint256 amount) external override returns (bool success) {
        _approve(msg.sender, spender, amount);
        return true;
    }

    /**
     * @notice Returns the allowance of a spender for an owner
     * @param owner Token owner
     * @param spender Approved spender
     * @return allowance Approved amount
     */
    function allowance(address owner, address spender) external view override returns (uint256) {
        return _allowances[owner][spender];
    }

    /**
     * @notice Mints tokens to an address (only if minter is set)
     * @param to Recipient address
     * @param amount Amount to mint
     * @return success True if mint succeeded
     */
    function mint(address to, uint256 amount) external override returns (bool success) {
        if (_minter == address(0)) {
            revert NotMintable();
        }
        if (msg.sender != _minter) {
            revert MinterMismatch(msg.sender, _minter);
        }
        if (_paused) {
            revert EnforcedPause();
        }
        if (to == address(0)) {
            revert InvalidReceiver(to);
        }

        _totalSupply += amount;
        unchecked {
            _balances[to] += amount;
        }
        emit Transfer(address(0), to, amount);
        return true;
    }

    /**
     * @notice Pauses token transfers (only if pauser is set)
     * @return success True if pause succeeded
     */
    function pause() external override returns (bool success) {
        if (_pauser == address(0)) {
            revert NotPausable();
        }
        if (msg.sender != _pauser) {
            revert PauserMismatch(msg.sender, _pauser);
        }
        if (_paused) {
            revert EnforcedPause();
        }

        _paused = true;
        emit Paused(msg.sender);
        return true;
    }

    /**
     * @notice Unpauses token transfers (only if pauser is set)
     * @return success True if unpause succeeded
     */
    function unpause() external override returns (bool success) {
        if (_pauser == address(0)) {
            revert NotPausable();
        }
        if (msg.sender != _pauser) {
            revert PauserMismatch(msg.sender, _pauser);
        }
        if (!_paused) {
            revert ExpectedPause();
        }

        _paused = false;
        emit Unpaused(msg.sender);
        return true;
    }

    /**
     * @notice Internal function to transfer tokens
     * @param from Source address
     * @param to Recipient address
     * @param amount Amount to transfer
     */
    function _transfer(address from, address to, uint256 amount) internal {
        if (from == address(0)) {
            revert InvalidSender(from);
        }
        if (to == address(0)) {
            revert InvalidReceiver(to);
        }
        if (_paused) {
            revert EnforcedPause();
        }

        uint256 fromBalance = _balances[from];
        if (fromBalance < amount) {
            revert InsufficientBalance(from, fromBalance, amount);
        }

        unchecked {
            _balances[from] = fromBalance - amount;
            _balances[to] += amount;
        }

        emit Transfer(from, to, amount);
    }

    /**
     * @notice Internal function to approve spending
     * @param owner Token owner
     * @param spender Approved spender
     * @param amount Amount to approve
     */
    function _approve(address owner, address spender, uint256 amount) internal {
        _allowances[owner][spender] = amount;
        emit Approval(owner, spender, amount);
    }
}
