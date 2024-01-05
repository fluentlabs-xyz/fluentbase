const { expect } = require("chai");
const { BigNumber } = require("ethers");

describe("Bridge", function () {
    let bridge;

    before(async function () {
        const BridgeContract = await ethers.getContractFactory("Bridge");
        const accounts = await hre.ethers.getSigners();

        bridge = await BridgeContract.deploy(accounts[0].address);
        await bridge.deployed();
    });

    it("Send message test", async function () {
        const accounts = await hre.ethers.getSigners();
        const contractWithSigner = bridge.connect(accounts[0]);
        const orgign_bridge_balance = await hre.ethers.provider.getBalance(bridge.address);

        const send_tx = await contractWithSigner.sendMessage(
            "0x1111111111111111111111111111111111111111",
            [1, 2, 3, 4, 5],
            { value: 100 },
        );

        await send_tx.wait();

        const events = await bridge.queryFilter("SentMessage", send_tx.blockNumber);

        expect(events.length).to.equal(1);

        expect(events[0].args.sender).to.equal(await accounts[0].getAddress());

        const bridge_balance = await hre.ethers.provider.getBalance(bridge.address);

        expect(bridge_balance.sub(orgign_bridge_balance)).to.be.eql(BigNumber.from(100));
    });

    it("Receive message test", async function () {
        const accounts = await hre.ethers.getSigners();
        const contractWithSigner = bridge.connect(accounts[0]);

        const receiverAddress = await accounts[1].getAddress();

        const origin_balance = await hre.ethers.provider.getBalance(receiverAddress);

        const receive_tx = await contractWithSigner.receiveMessage(
            "0x1111111111111111111111111111111111111111",
            receiverAddress,
            100,
            0,
            [],
        );

        await receive_tx.wait();

        const events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
            "0x1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        const new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(100));

        try {
            const repeat_receive_tx = await contractWithSigner.receiveMessage(
                "0x1111111111111111111111111111111111111111",
                receiverAddress,
                100,
                0,
                [],
            );

            await repeat_receive_tx.wait();
        } catch (error) {
            expect(error.toString()).to.equal(
                "Error: VM Exception while processing transaction: " +
                    "reverted with reason string 'Message already received'",
            );
        }
    });
});
