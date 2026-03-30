#!/usr/bin/env node

import fs from "node:fs";
import process from "node:process";
import {JsonRpcProvider, keccak256, Transaction} from "ethers";
import {encode as rlpEncode} from "rlp";

function usage() {
    console.error(
        "Usage: node gen_fixture.mjs <rpc_url> <tx_hash> [output.json]"
    );
    process.exit(1);
}

function ensureHex(value, fallback = "0x0") {
    if (typeof value == "number") {
        return '0x' + value.toString(16);
    }
    return typeof value === "string" && value.startsWith("0x") ? value : fallback;
}

function normalizeAccount(acc = {}) {
    return {
        balance: ensureHex(acc.balance, "0x0"),
        nonce: ensureHex(acc.nonce, "0x0"),
        code: typeof acc.code === "string" ? acc.code : "0x",
        storage: acc.storage && typeof acc.storage === "object" ? acc.storage : {},
    };
}

function mergeAccounts(preAcc = {}, postAcc = {}) {
    const pre = normalizeAccount(preAcc);
    const post = normalizeAccount(postAcc);

    return {
        balance: postAcc.balance !== undefined ? post.balance : pre.balance,
        nonce: postAcc.nonce !== undefined ? post.nonce : pre.nonce,
        code: postAcc.code !== undefined ? post.code : pre.code,
        storage: {
            ...pre.storage,
            ...post.storage,
        },
    };
}

function stripUndefined(obj) {
    if (Array.isArray(obj)) {
        return obj.map(stripUndefined);
    }
    if (obj && typeof obj === "object") {
        return Object.fromEntries(
            Object.entries(obj)
                .filter(([, v]) => v !== undefined)
                .map(([k, v]) => [k, stripUndefined(v)])
        );
    }
    return obj;
}

function logsHash(logs) {
    const encodedLogs = logs.map(log => [
        log.address,
        log.topics,
        log.data
    ]);
    const rlp = rlpEncode(encodedLogs);
    return keccak256(rlp);
}

async function main() {
    const [, , rpcUrl, txHash, outputFileArg] = process.argv;
    if (!rpcUrl || !txHash) usage();

    const outputFile = outputFileArg ?? `testcases/fixture_${txHash.slice(2, 10)}.json`;
    const provider = new JsonRpcProvider(rpcUrl);

    const tx = await provider.send("eth_getTransactionByHash", [txHash]);
    if (!tx) {
        throw new Error(`Transaction not found: ${txHash}`);
    }

    const receipt = await provider.send("eth_getTransactionReceipt", [txHash]);
    if (!receipt) {
        throw new Error(`Receipt not found: ${txHash}`);
    }

    const block =
        tx.blockHash != null
            ? await provider.send("eth_getBlockByHash", [tx.blockHash, false])
            : null;

    if (!block) {
        throw new Error(`Block not found for tx: ${txHash}`);
    }

    const preStateTrace = await provider.send("debug_traceTransaction", [
        txHash,
        {
            tracer: "prestateTracer",
            tracerConfig: {
                preStateMode: true,
            },
        },
    ]);

    const diffTrace = await provider.send("debug_traceTransaction", [
        txHash,
        {
            tracer: "prestateTracer",
            tracerConfig: {
                diffMode: true,
            },
        },
    ]);

    const tracePre = preStateTrace ?? {};
    const tracePost = diffTrace?.post ?? {};

    const preState = {};
    for (const [addr, acc] of Object.entries(tracePre)) {
        preState[addr] = normalizeAccount(acc);
    }

    const allTouched = new Set([
        ...Object.keys(tracePre),
        ...Object.keys(tracePost),
    ]);

    const postState = {};
    for (const addr of allTouched) {
        postState[addr] = mergeAccounts(tracePre[addr], tracePost[addr]);
    }

    // Remove fee manager (it's bytecode is not executed unless runtime upgrade)
    delete preState['0x0000000000000000000000000000000000520fee'];
    delete postState['0x0000000000000000000000000000000000520fee'];

    let rawTx;
    try {
        rawTx = Transaction.from({
            type: tx.type,
            to: tx.to,
            nonce: tx.nonce,
            gasLimit: tx.gas,
            gasPrice: tx.gasPrice,
            maxPriorityFeePerGas: tx.maxPriorityFeePerGas,
            maxFeePerGas: tx.maxFeePerGas,
            value: tx.value,
            data: tx.input,
            chainId: tx.chainId,
            accessList: tx.accessList,
            blobVersionedHashes: tx.blobVersionedHashes,
            maxFeePerBlobGas: tx.maxFeePerBlobGas,
            v: tx.v,
            r: tx.r,
            s: tx.s,
            yParity: tx.yParity,
        }).serialized;
    } catch {
        rawTx = undefined;
    }

    const chainId = await provider.send("eth_chainId", []);

    const logsHashValue = logsHash(receipt.logs);

    const fixture = stripUndefined({
        [`tx_${txHash.slice(2, 10)}`]: {
            _info: {
                comment: "Auto-generated from RPC using eth_getTransaction*, eth_getBlock*, and debug_traceTransaction(prestateTracer diffMode=true).",
                sourceTxHash: txHash,
                sourceBlockHash: tx.blockHash,
                sourceBlockNumber: tx.blockNumber,
            },

            config: {
                chainid: chainId,
            },

            env: {
                currentBaseFee: ensureHex(block.baseFeePerGas, "0x0"),
                currentCoinbase: block.miner ?? block.author ?? "0x0000000000000000000000000000000000000000",
                currentDifficulty: ensureHex(block.difficulty, "0x0"),
                currentGasLimit: ensureHex(block.gasLimit, "0x0"),
                currentNumber: ensureHex(block.number, "0x0"),
                currentRandom: ensureHex(block.mixHash ?? block.prevRandao, "0x0"),
                currentTimestamp: ensureHex(block.timestamp, "0x0"),
            },

            pre: preState,

            post: {
                Prague: [
                    {
                        hash: "0x0000000000000000000000000000000000000000000000000000000000000000",
                        indexes: {
                            data: 0,
                            gas: 0,
                            value: 0,
                        },
                        logs: logsHashValue,
                        state: postState,
                        txbytes: rawTx,
                    },
                ],
            },

            transaction: {
                accessLists: [tx.accessList ?? []],
                data: [tx.input ?? "0x"],
                gasLimit: [tx.gas],
                gasPrice: tx.gasPrice,
                maxFeePerGas: tx.maxFeePerGas,
                maxPriorityFeePerGas: tx.maxPriorityFeePerGas,
                nonce: tx.nonce,
                sender: tx.from,
                ...(tx.to ? {to: tx.to} : {}),
                value: [tx.value],
                secretKey: "0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8",
            },
        },
    });

    fs.writeFileSync(outputFile, JSON.stringify(fixture, null, 2));
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});