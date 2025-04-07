# EVM

This project provides an implementation of an execution environment that interfaces with the EVM or executes
WebAssembly-based smart contracts. It manages interactions between smart contracts and their
operational environment, handling deployment, execution, gas costs, and storage synchronization. The primary focus is on
ensuring compatibility with Ethereum standards while enabling seamless contract workflows.

The primary entry points of the codebase are the **`main`** and **`deploy`** functions. These drive the core logic for
executing smart contracts and deploying them into the blockchain-like environment.

## **Deploy**

The **`deploy`** function is used to deploy smart contracts. It includes setting initial storage or balances depending
on the specific deployment requirements.

1. **Fetch Input**:
    - Read input data (typically the smart contract's initialization bytecode or parameters).

2. **Validation**:
    - Perform checks such as:
        - EVM-specific limits like code size restrictions (**EIP-170**).
        - Input validity or compliance with a protocol like rejecting bytecodes starting with `0xEF`.

3. **Execution**:
    - Execute the bytecode using the EVM.
    - If execution fails, terminate the deployment early.

4. **Storage/State Updates**:
    - If execution is successful:
        - Store deployed bytecode.
        - Record initial balances or other state data using the SDK.

## **Main**

The **`main`** function is the entry point for executing contract bytecode. It handles reading input data, managing gas,
and writing results back after execution.

1. **Contextual Setup**:
    - Retrieve the gas limit via the SDK.
    - Calculate the gas cost of the incoming transaction based on the input size.

2. **Gas Check**:
    - If the required gas exceeds the available gas limit, the execution terminates with an **OutOfFuel** error.

3. **Execution**:
    - Input data is read and processed.
    - The function writes the processed data (in this case, identical to the input) back to the output.

---

P.S:
Tha EVM interpreter is based on modified revm's interpreter with replaced system calls and adjusted gas calculation
policy