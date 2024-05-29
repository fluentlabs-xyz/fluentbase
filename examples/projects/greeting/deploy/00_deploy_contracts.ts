import { HardhatRuntimeEnvironment } from "hardhat/types";
import { DeployFunction, DeploymentSubmission } from "hardhat-deploy/types";
import fs from "fs";
import path from "path";
import crypto from "crypto";

require("dotenv").config();

const DEPLOYER_PRIVATE_KEY =
    process.env.DEPLOYER_PRIVATE_KEY || "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const func: DeployFunction = async function (hre: HardhatRuntimeEnvironment) {
    const { ethers, network, deployments } = hre;
    const { save, getOrNull } = deployments;

    console.log("Deploying WASM contract...");
    const wasmBinaryPath = "./bin/greeting.wasm"; // TODO: Update this path to your actual wasm file
    const wasmBinary = fs.readFileSync(wasmBinaryPath);
    const wasmBinaryHash = crypto.createHash("sha256").update(wasmBinary).digest("hex");
    const artifactName = path.basename(wasmBinaryPath, ".wasm");

    const existingDeployment = await getOrNull(artifactName);

    if (existingDeployment && existingDeployment.metadata === wasmBinaryHash) {
        console.log(`WASM contract bytecode has not changed. Skipping deployment.`);
        console.log(`Existing contract address: ${existingDeployment.address}`);
        return;
    }

    // @ts-ignore
    const provider = new ethers.JsonRpcProvider(network.config.url);
    const deployer = new ethers.Wallet(DEPLOYER_PRIVATE_KEY, provider);
    const gasPrice = (await provider.getFeeData()).gasPrice;

    const transaction = {
        data: "0x" + wasmBinary.toString("hex"),
        gasLimit: 300_000_000,
        gasPrice: gasPrice,
    };

    const tx = await deployer.sendTransaction(transaction);
    const receipt = await tx.wait();

    if (receipt && receipt.contractAddress) {
        console.log(`WASM contract deployed at: ${receipt.contractAddress}`);

        const artifact = {
            abi: [], // Since there's no ABI for the WASM contract
            bytecode: "0x" + wasmBinary.toString("hex"),
            deployedBytecode: "0x" + wasmBinary.toString("hex"),
            metadata: wasmBinaryHash,
        };

        const deploymentData = {
            address: receipt.contractAddress,
            ...artifact,
        };

        await save(artifactName, deploymentData);
    } else {
        throw new Error("Failed to deploy WASM contract");
    }
};

export default func;
func.tags = ["all"];
