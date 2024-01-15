/** @type import('hardhat/config').HardhatUserConfig */
require("@nomiclabs/hardhat-ethers");
require("@nomiclabs/hardhat-solhint");

module.exports = {
  solidity: "0.8.20",
  networks: {
    L1: {
      url: "http://localhost:8545",
    },
    L2: {
      url: "http://localhost:8546",
    },  },
};
