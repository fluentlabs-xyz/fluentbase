const { ethers } = require("hardhat");

async function main() {
    const [deployer] = await ethers.getSigners();
    console.log(" Deploying contract with the account:", deployer.address);

    // Capture environment variables
    const tokenName = process.env.TOKEN_NAME;
    const tokenSymbol = process.env.TOKEN_SYMBOL;
    const tokenSupply = process.env.TOTAL_SUPPLY;

    if (!tokenName || !tokenSymbol || !tokenSupply) {
        console.error(" Error: Token name, symbol, and total supply are required!");
        process.exit(1);
    }

    console.log(`📜 Creating the contract with the following details:
➡ Name: ${tokenName}
➡ Symbol: ${tokenSymbol}
➡ Total Supply: ${tokenSupply} units`);

    // Deploy the contract
    const Token = await ethers.getContractFactory("Token");
    const token = await Token.deploy(tokenName, tokenSymbol, tokenSupply);

    await token.deployed(); // 🔥 Fixed here

    console.log(" Token deployed successfully!");
    console.log(" Contract address:", token.address);
}

main().catch((error) => {
    console.error(" Error during deployment:", error);
    process.exit(1);
});

