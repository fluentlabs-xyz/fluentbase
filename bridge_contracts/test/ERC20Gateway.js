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
    const BridgeContract = await ethers.getContractFactory("Bridge");
    const accounts = await hre.ethers.getSigners();
    bridge = await BridgeContract.deploy(accounts[0].address);
    await bridge.deployed();

    const TokenFactoryContract = await ethers.getContractFactory("ERC20TokenFactory");
    tokenFactory = await TokenFactoryContract.deploy(accounts[0].address);
    await tokenFactory.deployed();

    const ERC20GatewayContract =
      await ethers.getContractFactory("ERC20Gateway");
    erc20GatewayAbi = ERC20GatewayContract.interface.format();
    erc20Gateway = await ERC20GatewayContract.deploy(bridge.address, tokenFactory.address, {
      value: ethers.utils.parseEther("1000"),
    });





    Token = await ethers.getContractFactory("MockERC20Token");
    token = await Token.deploy(
      "Mock Token",
      "TKN",
      ethers.utils.parseEther("1000000"),
      accounts[0].address,
    ); // Adjust initial supply as needed
    await token.deployed();

    await erc20Gateway.deployed();

    const contractWithSigner = erc20Gateway.connect(accounts[0]);

    const updateMappingTx = await contractWithSigner.updateTokenMapping(
        token.address,
        "0x1111111111111111111111111111111111111111",
    );

    await updateMappingTx.wait();
  });

  it("Send tokens test", async function () {
    const accounts = await hre.ethers.getSigners();
    const tokenWithSigner = token.connect(accounts[0]);
    const approve_tx = await tokenWithSigner.approve(erc20Gateway.address, 100);
    await approve_tx.wait();

    const contractWithSigner = erc20Gateway.connect(accounts[0]);
    const origin_balance = await token.balanceOf(accounts[0].address);
    const origin_bridge_balance = await token.balanceOf(erc20Gateway.address);

    const send_tx = await contractWithSigner.sendTokens(
      token.address,
      accounts[3].address,
      100,
      []
    );

    await send_tx.wait();

    const events = await bridge.queryFilter("SentMessage", send_tx.blockNumber);

    expect(events.length).to.equal(1);

    expect(events[0].args.sender).to.equal(erc20Gateway.address);

    const balance = await token.balanceOf(accounts[0].address);
    const bridge_balance = await token.balanceOf(erc20Gateway.address);

    expect(bridge_balance.sub(origin_bridge_balance)).to.be.eql(
      BigNumber.from(100),
    );
    expect(origin_balance.sub(balance)).to.be.eql(BigNumber.from(100));
  });

  it("Receive tokens test", async function () {
    const accounts = await hre.ethers.getSigners();
    const contractWithSigner = bridge.connect(accounts[0]);

    const receiverAddress = await accounts[3].getAddress();

    const origin_balance =
      await hre.ethers.provider.getBalance(receiverAddress);
    const origin_bridge_balance = await token.balanceOf(erc20Gateway.address);
    const origin_token_balance = await token.balanceOf(receiverAddress);

    const gatewayInterface = new ethers.utils.Interface(erc20GatewayAbi);
    const _token = token.address;
    const _to = accounts[3].address;
    const _from = accounts[0].address;
    const _amount = 100;

    // Manually encode function call with the function selector
    const functionSelector = gatewayInterface.getSighash(
      "receiveTokens(address,address,address,uint256)",
    );
    const _message =
      functionSelector +
      ethers.utils.defaultAbiCoder
        .encode(
          ["address", "address", "address", "uint256"],
          [_token, _from, _to, _amount],
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

    expect(hash).to.equal("0x01221c10f2d58fd84042c898522a7689c3b83d3f36819c799eeaf4d328952a9d");

    const receive_tx = await contractWithSigner.receiveMessage(
      "0x1111111111111111111111111111111111111111",
      erc20Gateway.address,
      0,
      0,
      _message,
    );

    await receive_tx.wait();

    events = await bridge.queryFilter(
      "ReceivedMessage",
      receive_tx.blockNumber,
    );

    expect(events.length).to.equal(1);
    expect(events[0].args.messageHash).to.equal(
      "0x900a37d9fad7ad45c343d6d0fc2741b20e71ae2607393e6ef6e0dff3e799c60e",
    );
    expect(events[0].args.successfulCall).to.equal(true);

    const new_balance = await hre.ethers.provider.getBalance(receiverAddress);
    expect(new_balance.sub(origin_balance)).to.be.eql(BigNumber.from(0));

    const balance = await token.balanceOf(receiverAddress);
    const bridge_balance = await token.balanceOf(erc20Gateway.address);

    expect(origin_bridge_balance.sub(bridge_balance)).to.be.eql(
      BigNumber.from(100),
    );
    expect(balance.sub(origin_token_balance)).to.be.eql(BigNumber.from(100));

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
