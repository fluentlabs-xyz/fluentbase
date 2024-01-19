const { expect } = require("chai");

describe("Contract deployment and interaction", function () {
    let l1Token;
    let l1Gateway, l2Gateway;
    let l1Bridge, l2Bridge;
    let l1Url = 'http://127.0.0.1:8545/';
    let l2Url = 'http://127.0.0.1:8546/';
    let l1Implementation, l2Implementation;
    let rollup;

    let messageHashes = []

    before(async () => {
        [l1Gateway, l1Bridge, l1Implementation, l1Factory] = await SetUpChain(l1Url, true);

        [l2Gateway, l2Bridge, l2Implementation, l2Factory] = await SetUpChain(l2Url)

        let providerL1 = new ethers.providers.JsonRpcProvider(l1Url); // Replace with your node's RPC URL
        let signerL1 = providerL1.getSigner();
        const accounts = await hre.ethers.getSigners();

        const Token = await ethers.getContractFactory("MockERC20Token");
        l1Token = await Token.connect(signerL1).deploy("Mock Token", "TKN", ethers.utils.parseEther("1000000"), accounts[0].address); // Adjust initial supply as needed
        await l1Token.deployed();
        console.log("l1token: ", l1Token.address);

        console.log("L1 gw: ", l1Gateway.address, "L2 gw: ", l2Gateway.address);

        let tx = await l1Gateway.setOtherSide(l2Gateway.address, l2Implementation, l2Factory);
        await tx.wait()
        tx = await l2Gateway.setOtherSide(l1Gateway.address, l1Implementation, l1Factory);
        await tx.wait()
    });

    async function SetUpChain(provider_url, withRollup) {
        let provider = new ethers.providers.JsonRpcProvider(provider_url);

        let signer = provider.getSigner();

        const PeggedToken = await ethers.getContractFactory("ERC20PeggedToken");
        let peggedToken = await PeggedToken.connect(signer).deploy(); // Adjust initial supply as needed
        await peggedToken.deployed();
        console.log("Pegged token: ", peggedToken.address);

        const BridgeContract = await ethers.getContractFactory("Bridge");
        const accounts = await hre.ethers.getSigners();

        let rollupAddress = "0x0000000000000000000000000000000000000000";
        if (withRollup) {
            const RollupContract = await ethers.getContractFactory("Rollup");
            rollup = await RollupContract.connect(signer).deploy();
            rollupAddress = rollup.address;
            console.log("Rollup address: ", rollupAddress);
        }

        let bridge = await BridgeContract.connect(signer).deploy(accounts[0].address, rollupAddress);
        await bridge.deployed();
        console.log("Bridge: ", bridge.address);

        const TokenFactoryContract = await ethers.getContractFactory("ERC20TokenFactory");
        let tokenFactory = await TokenFactoryContract.connect(signer).deploy(peggedToken.address);
        await tokenFactory.deployed();
        console.log("TokenFactory: ", tokenFactory.address);

        const ERC20GatewayContract = await ethers.getContractFactory("ERC20Gateway");
        let erc20Gateway = await ERC20GatewayContract.connect(signer).deploy(bridge.address, tokenFactory.address, {
            value: ethers.utils.parseEther("1000"),
        });

        console.log("token factory owner: ", await tokenFactory.owner());
        const authTx = await tokenFactory.transferOwnership(erc20Gateway.address);
        await authTx.wait();
        console.log("token factory owner: ", await tokenFactory.owner());


        await erc20Gateway.deployed();
        console.log("Gateway: ", erc20Gateway.address);

        return [erc20Gateway, bridge, peggedToken.address, tokenFactory.address];
    }

    it("Compare pegged token addresses", async function () {
        let t1 = await l1Gateway.computePeggedTokenAddress(l1Token.address);
        let t2 = await l2Gateway.computeOtherSidePeggedTokenAddress(l1Token.address);
        expect(t1).to.equal(t2);
    });

    it("Bridging tokens between to contracts", async function () {
        let provider = new ethers.providers.JsonRpcProvider(l1Url);
        let accounts = await provider.listAccounts();

        const approve_tx = await l1Token.approve(l1Gateway.address, 100);
        await approve_tx.wait();

        console.log("Provider", l1Gateway.provider);

        console.log("Token send");
        const send_tx = await l1Gateway.sendTokens(l1Token.address, l2Gateway.signer.getAddress(), 100);
        console.log("Token sent", l1Token.address);
        await send_tx.wait();

        const events = await l1Bridge.queryFilter("SentMessage", send_tx.blockNumber);

        expect(events.length).to.equal(1);

        const sentEvent = events[0];

        const receive_tx = await l2Bridge.receiveMessage(
            sentEvent.args["sender"],
            sentEvent.args["to"],
            sentEvent.args["value"],
            sentEvent.args["nonce"],
            sentEvent.args["data"]
        );

        await receive_tx.wait();

        const bridge_events = await l2Bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);
        const error_events = await l2Bridge.queryFilter("Error", receive_tx.blockNumber);
        const gateway_events = await l2Gateway.queryFilter("ReceivedTokens", receive_tx.blockNumber);

        console.log("Bridge events: ", bridge_events);
        console.log("Error events: ", error_events);
        expect(error_events.length).to.equal(0);
        expect(bridge_events.length).to.equal(1);
        console.log("Gateway events: ", gateway_events);
        expect(gateway_events.length).to.equal(1);


        let peggedToken = await l2Gateway.computePeggedTokenAddress(l1Token.address);
        console.log("Pegged tokens: ", peggedToken);
        const sendBackTx = await l2Gateway.sendTokens(peggedToken, accounts[3], 100);
        console.log("Token sent", l1Token.address);
        await sendBackTx.wait();

        const backEvents = await l2Bridge.queryFilter("SentMessage", send_tx.blockNumber);

        expect(backEvents.length).to.equal(1);
        let messageHash = backEvents[0].args.messageHash;

        console.log(backEvents)
        const sentBackEvent = backEvents[0];

        const accept = await rollup.acceptNextProof(
            1,
            messageHash,
            []
        )

        await accept.wait();

        const receiveBackTx = await l1Bridge.receiveMessageWithProof(
            sentBackEvent.args["sender"],
            sentBackEvent.args["to"],
            sentBackEvent.args["value"],
            sentBackEvent.args["nonce"],
            sentBackEvent.args["data"],
            []
        );

        await receiveBackTx.wait();

        const bridgeBackEvents = await l1Bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);
        const errorBackEvents = await l1Bridge.queryFilter("Error", receive_tx.blockNumber);
        const gatewayBackEvents = await l1Gateway.queryFilter("ReceivedTokens", receive_tx.blockNumber);

        console.log("Bridge back events: ", bridgeBackEvents);
        console.log("Error back events: ", errorBackEvents);
        expect(errorBackEvents.length).to.equal(0);
        expect(bridgeBackEvents.length).to.equal(1);
        console.log("Gateway back events: ", gatewayBackEvents);
        expect(gatewayBackEvents.length).to.equal(1);
    });
});
