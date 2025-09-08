# Fluentbase EVM Tutorial

## Introduction

Fluentbase allows you to run EVM contracts seamlessly alongside WASM and SVM contracts. This tutorial guides you through deploying a simple Solidity contract on Fluentbase.

## Prerequisites

* Rust & Cargo installed: [Rust installation](https://www.rust-lang.org/tools/install)
* Fluentbase cloned and built:

```bash
git clone https://github.com/fluentlabs-xyz/fluentbase.git
cd fluentbase
cargo build --release
```

* Node.js & npm installed (for Solidity compilation using Hardhat)
* Basic knowledge of Solidity

## Step 1: Write a Simple EVM Contract

Create a file `SimpleStorage.sol`:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract SimpleStorage {
    uint256 private data;

    function set(uint256 _data) public {
        data = _data;
    }

    function get() public view returns (uint256) {
        return data;
    }
}
```

## Step 2: Compile the Contract

Use Hardhat or another Solidity compiler:

```bash
npx hardhat compile
```

You should see a compiled artifact in `artifacts/contracts/SimpleStorage.sol/SimpleStorage.json`.

## Step 3: Deploy on Fluentbase

1. Start Fluentbase local node (example command, adjust per your setup):

```bash
cargo run --release --bin fluentbase-node
```

2. Deploy the contract using Fluentbase CLI (example):

```bash
fluentbase deploy --vm evm ./artifacts/contracts/SimpleStorage.sol/SimpleStorage.json
```

3. Note the **contract address** printed after deployment.

## Step 4: Interact with the Contract

Set a value:

```bash
fluentbase call --address <CONTRACT_ADDRESS> --function set --args 42
```

Retrieve the value:

```bash
fluentbase call --address <CONTRACT_ADDRESS> --function get
```

You should see `42` as the returned result.

## Step 5: Troubleshooting Tips

* Ensure the local node is running before deployment.
* Check Solidity version compatibility (`pragma solidity ^0.8.0`).
* For any errors in compilation or deployment, refer to the Fluentbase docs: [Fluent Documentation](https://docs.fluent.xyz/)

## Step 6: Next Steps

* Try deploying a more complex contract (token contract, voting contract).
* Experiment with **cross-VM calls** between EVM and WASM contracts.
* Document your experience and submit it as a PR to the Fluentbase repository.
