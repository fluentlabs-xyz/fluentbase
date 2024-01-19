const { expect } = require("chai");
describe("Rollup", function () {
    let rollup;

    before(async function () {

        const RollupContract = await ethers.getContractFactory("Rollup");
        rollup = await RollupContract.deploy();

    });

    it("Calculate merkle root", async function () {

        let tx = await rollup.calculateMerkleRoot(
            Buffer.from("1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75", 'hex')
        );

        expect(tx).to.eq("0x1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75");

        tx = await rollup.calculateMerkleRoot(
            Buffer.from("1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac753e13975f9e4165cf4119f2f82528f20d0ba7d1ab18cf62b0e07a625fdcb600ba", 'hex')
        );

        expect(tx).to.eq("0xc40056c5e162e060269929562bcfe7c13a1a3f1cea0287e768c5f5099e0f9782");

        tx = await rollup.calculateMerkleRoot(
            Buffer.from("1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac753e13975f9e4165cf4119f2f82528f20d0ba7d1ab18cf62b0e07a625fdcb600ba6bb3a22ed7bf22ee8607e5c6afad2b02dde06fe81be5723452da97b74b162c87", 'hex')
        );

        expect(tx).to.eq("0x8fece7804bc45cca2731a7da2d68e4c1bd535996aecbdb3bfbbbcdb5c86ef5cb");

    });

    it("Accept proof", async function () {
        const accounts = await hre.ethers.getSigners();
        const rollupContractWithSigner = rollup.connect(accounts[0]);

        await rollupContractWithSigner.acceptNextProof(
            1,
            "0x1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75",
            []
            // Buffer.from("308e8f517a141f7661301a6f3c8a29ad2736bda89bf06403ead102d2075c2981", "hex")
            // [0x9b, 0x00, 0x34, 0x2a, 0x7f, 0xac, 0x9c, 0x0c, 0xc3, 0x3b, 0x36, 0x1c, 0xbb, 0x98, 0x43, 0x80, 0x33, 0x53, 0x77, 0x18, 0x1b, 0x94, 0x26, 0x47, 0xf4, 0x70, 0x58, 0xbc, 0x95, 0x12, 0x3d, 0x43]
        );
    })
});
