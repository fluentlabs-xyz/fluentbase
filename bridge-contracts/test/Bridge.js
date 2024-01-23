const { expect } = require("chai");
const { BigNumber } = require("ethers");

describe("Bridge", function () {
    let bridge;
    let rollup;

    before(async function () {

        const RollupContract = await ethers.getContractFactory("Rollup");
        rollup = await RollupContract.deploy();

        const BridgeContract = await ethers.getContractFactory("Bridge");
        const accounts = await hre.ethers.getSigners();

        bridge = await BridgeContract.deploy(accounts[0].address, rollup.address);
        await bridge.deployed();

        let setBridge = await rollup.setBridge(bridge.address);
        await setBridge.wait();
    });

    it("Send message test", async function () {
        const accounts = await hre.ethers.getSigners();
        const contractWithSigner = bridge.connect(accounts[0]);
        const orgign_bridge_balance = await hre.ethers.provider.getBalance(bridge.address);

        const send_tx = await contractWithSigner.sendMessage(
            "0x1111111111111111111111111111111111111111",
            [1, 2, 3, 4, 5],
            { value: 2000 },
        );

        await send_tx.wait();

        const events = await bridge.queryFilter("SentMessage", send_tx.blockNumber);

        expect(events.length).to.equal(1);

        expect(events[0].args.sender).to.equal(await accounts[0].getAddress());

        const bridge_balance = await hre.ethers.provider.getBalance(bridge.address);

        expect(bridge_balance.sub(orgign_bridge_balance)).to.be.eql(BigNumber.from(2000));
    });

    it("Receive message test", async function () {
        const accounts = await hre.ethers.getSigners();
        const contractWithSigner = bridge.connect(accounts[0]);

        const receiverAddress = await accounts[1].getAddress();

        const origin_balance = await hre.ethers.provider.getBalance(receiverAddress);

        const receive_tx = await contractWithSigner.receiveMessage(
            "0x1111111111111111111111111111111111111111",
            receiverAddress,
            200,
            0,
            [],
        );

        await receive_tx.wait();

        const events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
            "0x5e6af7e11771fafdbeba41d9781ea9a8fcdac0a801b5df4deebde301997fc061",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        const new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(200));

        try {
            const repeat_receive_tx = await contractWithSigner.receiveMessage(
                "0x1111111111111111111111111111111111111111",
                receiverAddress,
                200,
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

    it("Receive message with proof test", async function () {
        const accounts = await hre.ethers.getSigners();
        const rollupContractWithSigner = rollup.connect(accounts[0]);

        await rollupContractWithSigner.acceptNextProof(
            1,
            "0x1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75",
            []
        )

        const contractWithSigner = bridge.connect(accounts[0]);

        const receiverAddress = await accounts[1].getAddress();

        const origin_balance = await hre.ethers.provider.getBalance(receiverAddress);

        let receive_tx = await contractWithSigner.receiveMessageWithProof(
            "0x1111111111111111111111111111111111111111",
            receiverAddress,
            100,
            0,
            [],
            [],
            1
        );

        await receive_tx.wait();

        let events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
            "0x1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        let new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(100));

        await rollupContractWithSigner.acceptNextProof(
            2,
            "0x3e13975f9e4165cf4119f2f82528f20d0ba7d1ab18cf62b0e07a625fdcb600ba",
            []
        )

        receive_tx = await contractWithSigner.receiveMessageWithProof(
            "0x1111111111111111111111111111111111111111",
            receiverAddress,
            100,
            1,
            [],
            Buffer.from("1fbe8b16b467b65c93cc416c9f6a43585820a41b90f14f6b74abe46e017fac75", 'hex'),
            2
        );

        events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
            "0x835612469dd5d58ef5be0da80c826de8354bbdd63eec7aea2dcca10ab8c0ff73",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(200));

        await rollupContractWithSigner.acceptNextProof(
            3,
            "0xf205d0a2ae61551dafb4c8b459883c5ad295948069f23d97d9e2e5a21f02ab7b",
            []
        )

        receive_tx = await contractWithSigner.receiveMessageWithProof(
            "0x1111111111111111111111111111111111111111",
            receiverAddress,
            100,
            2,
            [],
            Buffer.from("00000000000000000000000000000000000000000000000000000000000000003e13975f9e4165cf4119f2f82528f20d0ba7d1ab18cf62b0e07a625fdcb600ba", 'hex'),
            3
        );

        events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
            "0x6bb3a22ed7bf22ee8607e5c6afad2b02dde06fe81be5723452da97b74b162c87",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(300));

    });
});
