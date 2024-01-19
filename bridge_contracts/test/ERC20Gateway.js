const { expect } = require("chai");
const { BigNumber } = require("ethers");
const { address } = require("hardhat/internal/core/config/config-validation");

describe("Bridge", function () {
    let bridge;
    let erc20Gateway;
    let erc20GatewayAbi;
    let token;
    let tokenFactory;

    before(async function () {
        const PeggedToken = await ethers.getContractFactory("ERC20PeggedToken");
        let peggedToken = await PeggedToken.deploy(); // Adjust initial supply as needed
        await peggedToken.deployed();

        const BridgeContract = await ethers.getContractFactory("Bridge");
        const accounts = await hre.ethers.getSigners();
        bridge = await BridgeContract.deploy(accounts[0].address, accounts[1].address);
        await bridge.deployed();

        const TokenFactoryContract = await ethers.getContractFactory("ERC20TokenFactory");
        tokenFactory = await TokenFactoryContract.deploy(peggedToken.address);
        await tokenFactory.deployed();

        const ERC20GatewayContract = await ethers.getContractFactory("ERC20Gateway");
        erc20GatewayAbi = ERC20GatewayContract.interface.format();
        erc20Gateway = await ERC20GatewayContract.deploy(bridge.address, tokenFactory.address, {
            value: ethers.utils.parseEther("1000"),
        });

        const authTx = await tokenFactory.transferOwnership(erc20Gateway.address);
        await authTx.wait();

        const Token = await ethers.getContractFactory("MockERC20Token");
        token = await Token.deploy("Mock Token", "TKN", ethers.utils.parseEther("1000000"), accounts[0].address); // Adjust initial supply as needed
        await token.deployed();

        await erc20Gateway.deployed();

        const contractWithSigner = erc20Gateway.connect(accounts[0]);
    });

    it("Send tokens test", async function () {
        const accounts = await hre.ethers.getSigners();
        const tokenWithSigner = token.connect(accounts[0]);
        const approve_tx = await tokenWithSigner.approve(erc20Gateway.address, 100);
        await approve_tx.wait();

        const contractWithSigner = erc20Gateway.connect(accounts[0]);
        const origin_balance = await token.balanceOf(accounts[0].address);
        const origin_bridge_balance = await token.balanceOf(erc20Gateway.address);

        const send_tx = await contractWithSigner.sendTokens(token.address, accounts[3].address, 100);

        await send_tx.wait();

        const events = await bridge.queryFilter("SentMessage", send_tx.blockNumber);

        expect(events.length).to.equal(1);

        expect(events[0].args.sender).to.equal(erc20Gateway.address);

        const balance = await token.balanceOf(accounts[0].address);
        const bridge_balance = await token.balanceOf(erc20Gateway.address);

        expect(bridge_balance.sub(origin_bridge_balance)).to.be.eql(BigNumber.from(100));
        expect(origin_balance.sub(balance)).to.be.eql(BigNumber.from(100));
    });

    it("Receive tokens test", async function () {
        const accounts = await hre.ethers.getSigners();
        const contractWithSigner = bridge.connect(accounts[0]);

        const receiverAddress = await accounts[3].getAddress();

        const origin_balance = await hre.ethers.provider.getBalance(receiverAddress);

        const gatewayInterface = new ethers.utils.Interface(erc20GatewayAbi);
        const _token = token.address;
        const _to = accounts[3].address;
        const _from = accounts[0].address;
        const _amount = 100;

        const functionSelector = gatewayInterface.getSighash(
            "receivePeggedTokens(address,address,address,address,uint256,bytes)",
        );

        const peggedTokenAddress = await tokenFactory.computePeggedTokenAddress(erc20Gateway.address, token.address);

        const tokenMetadata = {
            name: "MyToken",
            symbol: "MTK",
            decimals: 18,
        };

        const encodedTokenMetadata = ethers.utils.defaultAbiCoder.encode(
            ["string", "string", "uint8"],
            [tokenMetadata.symbol, tokenMetadata.name, tokenMetadata.decimals],
        );

        const _message =
            functionSelector +
            ethers.utils.defaultAbiCoder
                .encode(
                    ["address", "address", "address", "address", "uint256", "bytes"],
                    [_token, peggedTokenAddress, _from, _to, _amount, encodedTokenMetadata],
                )
                .slice(2);

        const data = hre.ethers.utils.defaultAbiCoder
            .encode(
                ["address", "address", "uint256", "uint256", "bytes"],
                [erc20Gateway.address, accounts[3].address, 0, 0, _message],
            )
            .slice(2);

        const inputBytes = Buffer.from(data, "hex");
        const hash = ethers.utils.keccak256(inputBytes);

        expect(hash).to.equal("0xed6c9db457d77d53b6bf21b449e6404456b3628853693ee133cb2e893e6cc503");

        const receive_tx = await contractWithSigner.receiveMessage(
            "0x1111111111111111111111111111111111111111",
            erc20Gateway.address,
            0,
            0,
            _message,
        );

        await receive_tx.wait();

        let error_events = await bridge.queryFilter("Error", receive_tx.blockNumber);

        expect(error_events.length).to.equal(0);
        let events = await bridge.queryFilter("ReceivedMessage", receive_tx.blockNumber);

        expect(events.length).to.equal(1);
        expect(events[0].args.messageHash).to.equal(
          "0x7523b9a58ac19b3ba6f272eb7120f22dcdaab28cc069fe8829acdba0e3992507",
        );
        expect(events[0].args.successfulCall).to.equal(true);

        const new_balance = await hre.ethers.provider.getBalance(receiverAddress);
        expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(0));

        const tokenArtifact = await artifacts.readArtifact("ERC20PeggedToken");
        const tokenAbi = tokenArtifact.abi;

        let peggedTokenContract = new ethers.Contract(peggedTokenAddress, tokenAbi, ethers.provider.getSigner());

        const balance = await peggedTokenContract.balanceOf(receiverAddress);

        expect(balance).to.be.eql(BigNumber.from(100));

        try {
            const repeat_receive_tx = await contractWithSigner.receiveMessage(
                "0x1111111111111111111111111111111111111111",
                erc20Gateway.address,
                0,
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
