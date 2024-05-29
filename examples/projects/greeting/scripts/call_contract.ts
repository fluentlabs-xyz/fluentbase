import { deployments, ethers } from "hardhat";

// Function to check if a string is ASCII
function isASCII(str: string) {
    return /^[\x00-\x7F]*$/.test(str);
}

async function main() {
    // Read the deployment artifact to get the contract address
    const artifactName = "greeting"; // Update this to the actual artifact name used during deployment
    const deployment = await deployments.get(artifactName);
    const contractAddress = deployment.address;

    // Initialize the provider and contract
    // @ts-ignore
    const provider = new ethers.JsonRpcProvider(network.config.url);
    const result = await provider.call({
        to: contractAddress,
    });

    // Convert the result from hex to ASCII and check if it's ASCII
    const hexToAscii = ethers.toUtf8String(result);
    if (isASCII(hexToAscii)) {
        console.log(`Message: "${hexToAscii}"`);
    } else {
        console.log(`Message: "${result}"`);
    }
}

// Execute the main function
main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
