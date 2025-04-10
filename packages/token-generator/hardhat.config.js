require("@nomiclabs/hardhat-ethers");
require("@nomicfoundation/hardhat-verify");

/**
 * @type import('hardhat/config').HardhatUserConfig
 */
module.exports = {
  networks: {
    fluent_devnet1: {
      url: "https://rpc.dev.gblend.xyz/", // RPC URL for Fluent Devnet
      chainId: 20993, // Chain ID for Fluent Devnet
      accounts: [
        `0x${"3336b3cc55879120f208a142c2b6471ca79e764074d7f5b583ddff02c5992fd2"}`,
      ], // Replace with the private key of the deploying account
    },
  },
  solidity: {
    version: "0.8.20", // Solidity compiler version
  },
  etherscan: {
    apiKey: "empty", // For Blockscout, leave as "empty"
    customChains: [
      {
        network: "fluent_devnet1",
        chainId: 20993,
        urls: {
          apiURL: "https://blockscout.dev.gblend.xyz/api",
          browserURL: "https://blockscout.dev.gblend.xyz",
        },
      },
    ],
  },
};

