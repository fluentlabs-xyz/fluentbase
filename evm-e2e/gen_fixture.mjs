#!/usr/bin/env node

import fs from "node:fs";
import process from "node:process";
import {JsonRpcProvider, Transaction, keccak256} from "ethers";
import {encode as rlpEncode} from "rlp";

function usage() {
    console.error(
        "Usage: node gen_fixture.js <rpc_url> <tx_hash> [output.json]"
    );
    process.exit(1);
}

function ensureHex(value, fallback = "0x0") {
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
    console.log(logs);
    const encodedLogs = logs.map(log => [
        log.address,
        log.topics,
        log.data
    ]);
    const rlp = rlpEncode(encodedLogs);
    const hash = keccak256(rlp);
    console.log(`hash: ${hash}`);
    return hash;
}

const GENESIS_EXCLUDE = new Set([
    // EVM precompiles
    "0x0000000000000000000000000000000000000001",
    "0x0000000000000000000000000000000000000002",
    "0x0000000000000000000000000000000000000003",
    "0x0000000000000000000000000000000000000004",
    "0x0000000000000000000000000000000000000005",
    "0x0000000000000000000000000000000000000006",
    "0x0000000000000000000000000000000000000007",
    "0x0000000000000000000000000000000000000008",
    "0x0000000000000000000000000000000000000009",
    "0x000000000000000000000000000000000000000a",
    "0x000000000000000000000000000000000000000b",
    "0x000000000000000000000000000000000000000c",
    "0x000000000000000000000000000000000000000d",
    "0x000000000000000000000000000000000000000e",
    "0x000000000000000000000000000000000000000f",
    "0x0000000000000000000000000000000000000010",
    "0x0000000000000000000000000000000000000011",

    // Fluent system runtimes
    "0x0000000000000000000000000000000000520001",
    "0x0000000000000000000000000000000000520003",
    "0x0000000000000000000000000000000000520008",
    "0x0000000000000000000000000000000000520009",

    // other system contracts
    "0x0000000000000000000000000000000000520010",
    "0x0000000000000000000000000000000000520011",
    "0x0000000000000000000000000000000000520012",
    "0x0000000000000000000000000000000000520fee",

    // EIPs
    "0x0000f90827f1c53a10cb7a02335b175320002935",
    "0x0000000000000000000000000000000000000100"
]);

function shouldInclude(addr) {
    return !GENESIS_EXCLUDE.has(addr.toLowerCase());
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

    const trace = await provider.send("debug_traceTransaction", [
        txHash,
        {
            tracer: "prestateTracer",
            tracerConfig: {
                diffMode: true,
            },
        },
    ]);

    const tracePre = trace?.pre ?? {};
    const tracePost = trace?.post ?? {};

    const preState = {};
    for (const [addr, acc] of Object.entries(tracePre)) {
        if (!shouldInclude(addr)) continue;
        preState[addr] = normalizeAccount(acc);
    }

    const allTouched = new Set([
        ...Object.keys(tracePre),
        ...Object.keys(tracePost),
    ]);

    const postState = {};
    for (const addr of allTouched) {
        if (!shouldInclude(addr)) continue;
        postState[addr] = mergeAccounts(tracePre[addr], tracePost[addr]);
    }

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
                        hash: tx.hash,
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
                nonce: 0, //tx.nonce,
                sender: tx.from,
                to: tx.to,
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