const web3 = require('web3');
const {Web3, ETH_DATA_FORMAT} = require('web3');
const {hexToBytes} = require('web3-utils');
const {ethRpcMethods} = require('web3-rpc-methods');
const {Wallet, Provider} = require('fuels');

const DEPLOYER_PRIVATE_KEY = 'ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';
const PRECOMPILE_FVM_ADDRESS = '0x0000000000000000000000000000000000005250';

function dec2hex(n) {
    let res = n ? [n % 256].concat(dec2hex(~~(n / 256))) : [];
    return res.reverse()
}

const EMPTY_STRING_CODE = 0x80;
const EMPTY_LIST_CODE = 0xC0;

class RlpHeader {

    constructor(is_list = false, payload_length = 0) {
        this.is_list = is_list
        this.payload_length = payload_length
    }

    encode() {
        let out = []
        if (this.payload_length < 56) {
            let code = EMPTY_STRING_CODE
            if (this.is_list) {
                code = EMPTY_LIST_CODE
            }
            out.push(code + self.payload_length)
        } else {
            let len_be = dec2hex(this.payload_length);
            let code = 0xB7;
            if (this.is_list) {
                code = 0xF7
            }
            out.push(code + len_be.length)
            out.push(...len_be)
        }
        return out
    }
}

const main = async () => {
    if (process.argv.length < 2) {
        console.log(`You must specify local or remote flag`);
        console.log(`Example: node send-blended.js --local`);
        process.exit(-1);
    }
    let args = process.argv.slice(2);
    const checkFlag = (param) => {
        let indexOf = args.indexOf(param)
        if (indexOf < 0) {
            return false
        }
        args.splice(indexOf, 1)
        return true
    };
    let isLocal = checkFlag('--local')
    let isDev = checkFlag('--dev')

    let web3Url = '';
    if (isLocal) {
        web3Url = 'http://127.0.0.1:8545';
    } else if (isDev) {
        web3Url = 'https://rpc.dev.thefluent.xyz/';
    } else {
        console.log(`You must specify --dev or --local flag!`);
        console.log(`Example: node deploy-contract.js --local`);
        process.exit(-1);
    }

    const web3 = new Web3(web3Url);

    // {
    //     const LOCAL_FUEL_NETWORK = 'http://127.0.0.1:4000/v1/graphql';
    //     const fuelProvider = await Provider.create(LOCAL_FUEL_NETWORK);
    //     let fuelEthAssetId = "0x0000000000000000000000000000000000000000000000000000000000000000";
    //     let fuelBaseAssetId = "0xf8f8b6283d7fa5b672b530cbb84fcccb4ff8dc40f8176ef4544ddb1f1952ad07";
    //
    //     let baseAssetId = fuelProvider.getBaseAssetId();
    //     console.log(`baseAssetId ${baseAssetId}`);
    //
    //     // let fuelTestWallet = await generateTestWallet(fuelProvider, [
    //     //     [42, baseAssetId],
    //     // ]);
    //     // let fuelTestWalletCoins = await fuelProvider.getCoins(fuelTestWallet.address);
    //     // console.log(`fuelTestWalletCoins`, fuelTestWalletCoins);
    //     // let fuelTestWalletBalance = await fuelTestWallet.getBalance();
    //     // console.log(`fuelWalletOfficialBalance ${fuelTestWalletBalance}`);
    //
    //     // let fuelSecretOfficial = "a1447cd75accc6b71a976fd3401a1f6ce318d27ba660b0315ee6ac347bf39568";
    //     // let fuelWalletOfficial = Wallet.fromPrivateKey(fuelSecretOfficial, fuelProvider);
    //
    //     let fuelSecretOfficial = "de97d8624a438121b86a1956544bd72ed68cd69f2c99555b08b1e8c51ffd511c";
    //     let fuelWalletOfficial = Wallet.fromPrivateKey(fuelSecretOfficial, fuelProvider);
    //     console.log(`fuelWalletOfficial.address`, fuelWalletOfficial.address.toHexString());
    //     let fuelWalletOfficialCoins = await fuelProvider.getCoins(fuelWalletOfficial.address);
    //     console.log(`fuelWalletOfficialCoins:`, fuelWalletOfficialCoins);
    //
    //     let fuelSecret1 = "0x99e87b0e9158531eeeb503ff15266e2b23c2a2507b138c9d1b1f2ab458df2d61";
    //     let fuelWallet1 = Wallet.fromPrivateKey(fuelSecret1, fuelProvider);
    //     console.log(`fuelWallet1.address:`, fuelWallet1.signer().address.toHexString());
    //     let fuelWallet1Coins = await fuelProvider.getCoins(fuelWallet1.address);
    //     console.log(`fuelWallet1Coins:`, fuelWallet1Coins);
    //
    //     // let fuelWallet2 = Wallet.fromAddress("0x53a9c6a74bee79c5e04115a007984f4bddaafed75f512f68766c6ed59d0aedec", fuelProvider);
    //     // console.log(`fuelWallet2.address:`, fuelWallet2.address.toHexString());
    //     // let fuelWallet2Coins = await fuelProvider.getCoins(fuelWallet2.address);
    //     // console.log(`fuelWallet2Coins:`, fuelWallet2Coins);
    //
    //     console.log("fuel: creating transfer");
    //     let fuelTransferFromOfficialToWallet1Tx = await fuelWalletOfficial.createTransfer(fuelWallet1.address, 1);
    //     console.log("fuelTransferFromOfficialToWallet1Tx:", fuelTransferFromOfficialToWallet1Tx);
    //     let transferResult = await fuelWallet1.sendTransaction(fuelTransferFromOfficialToWallet1Tx);
    //     console.log(`transferResult`, transferResult);
    //     let {id} = await transferResult.wait();
    //     console.log(`transfer id`, id);
    //
    //
    //     fuelWalletOfficialCoins = await fuelProvider.getCoins(fuelWalletOfficial.address);
    //     console.log(`fuelWalletOfficialCoins:`, fuelWalletOfficialCoins);
    //     fuelWallet1Coins = await fuelProvider.getCoins(fuelWallet1.address);
    //     console.log(`fuelWallet1Coins:`, fuelWallet1Coins);
    // }

    let doSendBalance = false;
    let doSendWrappedFuelTx = true;

    // witness address 32 (original) f5bd94297364b371180b42da 369f74918912b80c9947d6a174c0c6e2c95fae1d
    let fuelTxOwnerAddress = "0x369f74918912b80c9947d6A174c0C6e2c95fAe1D";
    let fuelTxOwnerAddressBalance = await web3.eth.getBalance(fuelTxOwnerAddress);
    console.log(`for fuelTxOwnerAddress ${fuelTxOwnerAddress} balance ${fuelTxOwnerAddressBalance}`);
    let privateKey = process.env.DEPLOYER_PRIVATE_KEY || DEPLOYER_PRIVATE_KEY;
    let account = web3.eth.accounts.privateKeyToAccount('0x' + privateKey);
    let accountBalance = await web3.eth.getBalance(account.address);
    console.log(`for account ${account.address} balance ${accountBalance}`);

    // send ETH to FUEL acc (fuel addr is a cut version)
    if (doSendBalance) {
        console.log(`sending balance to ${account.address}->${fuelTxOwnerAddress}`)
        const gasPrice = await web3.eth.getGasPrice(ETH_DATA_FORMAT);
        let ethAmountToSend = web3.utils.toWei(0.1, "ether");
        let rawTransaction = {
            from: account.address,
            gasPrice: gasPrice,
            gas: 300_000_000,
            to: fuelTxOwnerAddress,
            value: ethAmountToSend,
            // "chainId": 1337 // Remember to change this
        };
        console.log(`rawTransaction:`, rawTransaction)
        const signedTransaction = await web3.eth.accounts.signTransaction(rawTransaction, privateKey)
        await web3.eth.sendSignedTransaction(signedTransaction.rawTransaction)
            .on('confirmation', confirmation => {
                console.log(`confirmation:`, confirmation)
            });
        console.log(`balance sent`);
    }

    let fluentTxType = 0x52;
    let fuelExecEnv = 0x00;

    console.log('Forming transaction...');
    let fuelTxHexStr = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000008000000000000000100000000000000010000000000000001240400000000000000000000000000000000000000000000ca41dab08590eda44231b6fcf4bb110c852b24f030bf996a89f02cccc57eb5f10000000000006a13f5bd94297364b371180b42da369f74918912b80c9947d6a174c0c6e2c95fae1d0000000000000064000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000028f974f02ef30fe9ee3e62b50f24a56d9042b4bc0251f23248d92990408afa49f000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040f8f6ccbd3005a7900db1de6987e439e09c37ea2ac56eb6c2d82eeb37c3d6450bbf7514dceee2d94dc24f6f610d10f13fc4e6f6dbca6a0d3864b77a2bc7e6f384";
    let fuelTxBytes = Array.from(hexToBytes(fuelTxHexStr));
    let fuelTxHex = "0x" + Buffer.from(fuelTxBytes).toString("hex");
    console.log(`fuelTxHex ${fuelTxHex}`)
    console.log(`fuelTxHex.length ${fuelTxHex.length}`)

    // let fuelTxBytesHeader = new RlpHeader(false, fuelTxBytes.length);
    // let fuelTxBytesHeaderRlp = fuelTxBytesHeader.encode();
    // rawTxBytes.push(...fuelTxBytesHeaderRlp);
    // rawTxBytes.push(...fuelTxBytes);
    // rawTxBytes = [fuelExecEnv].concat(rawTxBytes)
    // let typedTxBytesHeader = new RlpHeader(true, rawTxBytes.length);
    // let typedTxBytesHeaderRlp = typedTxBytesHeader.encode();
    // rawTxBytes = typedTxBytesHeaderRlp.concat(rawTxBytes)
    // rawTxBytes = [fluentTxType].concat(rawTxBytes)
    // // expected: "52 f901b4 00 b901b0 000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000008000000000000000100000000000000010000000000000001240400000000000000000000000000000000000000000000ca41dab08590eda44231b6fcf4bb110c852b24f030bf996a89f02cccc57eb5f10000000000006a13f5bd94297364b371180b42da369f74918912b80c9947d6a174c0c6e2c95fae1d0000000000000064000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000028f974f02ef30fe9ee3e62b50f24a56d9042b4bc0251f23248d92990408afa49f000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040f8f6ccbd3005a7900db1de6987e439e09c37ea2ac56eb6c2d82eeb37c3d6450bbf7514dceee2d94dc24f6f610d10f13fc4e6f6dbca6a0d3864b77a2bc7e6f384";
    //
    // let rawTxHex = "0x" + Buffer.from(rawTxBytes).toString("hex");
    // console.log(`rawTxHex ${rawTxHex}`)
    // console.log(`rawTxBytes.length ${rawTxBytes.length}`)
    // // const signedTransaction = await web3.eth.accounts.signTransaction(rawTx, privateKey)

    let txHashHex = "0x0000000000000000000000000000000000000000000000000000000000000000";
    if (doSendWrappedFuelTx) {
        // try {
        console.log(`sending wrapped fuel tx`)
        const gasPrice = await web3.eth.getGasPrice(ETH_DATA_FORMAT);
        let ethAmountToSend = web3.utils.toWei(0.01, "ether");
        let rawTransaction = {
            from: account.address,
            gasPrice: gasPrice,
            gas: 300_000_000,
            to: PRECOMPILE_FVM_ADDRESS,
            input: fuelTxHex,
            // value: ethAmountToSend,
            // "chainId": 1337 // Remember to change this
        };
        console.log(`rawTransaction:`, rawTransaction)
        // const signedTransaction = await web3.eth.accounts.signTransaction(rawTransaction, privateKey)
        let res = await web3.eth.call(rawTransaction);
        // await web3.eth.sendSignedTransaction(signedTransaction.rawTransaction)
        //     .on('confirmation', confirmation => {
        //         console.log(`confirmation:`, confirmation)
        //     });
        console.log(`wrapped fuel tx sent`);
        // } catch (e) {
        //     console.log(`failed to sendRawTransaction: '${e}'`);
        // }
    }
    console.log(`txHashHex: ${txHashHex}`);
    try {
        let tx = await ethRpcMethods.getTransactionByHash(web3.requestManager, txHashHex);
        console.log(`getTransactionByHash: ${JSON.stringify(tx)}`);
    } catch (e) {
        console.log(`failed to getTransactionByHash: '${e}'`);
    }
    process.exit(0)
}

main().then(console.log).catch(console.error);
